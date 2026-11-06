use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use vt100::{MouseProtocolEncoding, MouseProtocolMode};

/// Encode a key for the shell. `None` means the key is swallowed.
pub fn encode_key(event: &KeyEvent, application_cursor: bool) -> Option<Vec<u8>> {
    if event.kind == KeyEventKind::Release {
        return None;
    }
    if matches!(
        event.code,
        KeyCode::Modifier(_)
            | KeyCode::CapsLock
            | KeyCode::ScrollLock
            | KeyCode::NumLock
            | KeyCode::PrintScreen
            | KeyCode::Pause
            | KeyCode::Menu
    ) {
        return None;
    }

    let mods = event.modifiers;
    match event.code {
        KeyCode::Char(ch) => encode_char(ch, mods),
        KeyCode::Null => Some(vec![0]),
        KeyCode::Esc => Some(prefix_alt(mods.contains(KeyModifiers::ALT), &[0x1b])),
        KeyCode::Enter => Some(prefix_alt(mods.contains(KeyModifiers::ALT), b"\r")),
        KeyCode::Tab | KeyCode::BackTab => {
            if event.code == KeyCode::BackTab || mods.contains(KeyModifiers::SHIFT) {
                Some(b"\x1b[Z".to_vec())
            } else {
                Some(prefix_alt(mods.contains(KeyModifiers::ALT), b"\t"))
            }
        }
        KeyCode::Backspace => {
            if mods.contains(KeyModifiers::CONTROL) {
                Some(prefix_alt(mods.contains(KeyModifiers::ALT), &[0x17]))
            } else {
                Some(prefix_alt(mods.contains(KeyModifiers::ALT), &[0x7f]))
            }
        }
        KeyCode::Up => Some(encode_arrow(b'A', mods, application_cursor)),
        KeyCode::Down => Some(encode_arrow(b'B', mods, application_cursor)),
        KeyCode::Right => Some(encode_arrow(b'C', mods, application_cursor)),
        KeyCode::Left => Some(encode_arrow(b'D', mods, application_cursor)),
        KeyCode::Home => Some(encode_home_end(b'H', mods, application_cursor)),
        KeyCode::End => Some(encode_home_end(b'F', mods, application_cursor)),
        KeyCode::Delete => Some(encode_tilde(3, mods)),
        KeyCode::Insert => Some(encode_tilde(2, mods)),
        KeyCode::PageUp => Some(encode_tilde(5, mods)),
        KeyCode::PageDown => Some(encode_tilde(6, mods)),
        KeyCode::F(n) => encode_function(n, mods),
        KeyCode::KeypadBegin => Some(b"\x1b[E".to_vec()),
        _ => None,
    }
}

pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
    }
    out.extend_from_slice(text.as_bytes());
    if bracketed {
        out.extend_from_slice(b"\x1b[201~");
    }
    out
}

/// `column` and `row` are 0-based coordinates inside the shell viewport.
pub fn encode_mouse(
    kind: MouseEventKind,
    modifiers: KeyModifiers,
    column: u16,
    row: u16,
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    if mode == MouseProtocolMode::None {
        return None;
    }
    let (button, action) = mouse_action(kind)?;
    if !mode_allows(mode, action, button) {
        return None;
    }
    // SGR reports the button number and uses `m` for release. The legacy
    // encodings use button code 3 for release instead.
    let base = if matches!(action, MouseAction::Release)
        && !matches!(encoding, MouseProtocolEncoding::Sgr)
    {
        3
    } else {
        button
    };
    let code = base.saturating_add(modifier_bits(modifiers));
    let x = column.checked_add(1)?;
    let y = row.checked_add(1)?;
    match encoding {
        MouseProtocolEncoding::Sgr => {
            Some(format!("\x1b[<{code};{x};{y}{}", sgr_suffix(action)).into_bytes())
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            encode_legacy_mouse(code, x, y, encoding)
        }
    }
}

#[derive(Clone, Copy)]
enum MouseAction {
    Press,
    Release,
    Motion,
    Wheel,
}

fn mouse_action(kind: MouseEventKind) -> Option<(u8, MouseAction)> {
    Some(match kind {
        MouseEventKind::Down(MouseButton::Left) => (0, MouseAction::Press),
        MouseEventKind::Down(MouseButton::Middle) => (1, MouseAction::Press),
        MouseEventKind::Down(MouseButton::Right) => (2, MouseAction::Press),
        MouseEventKind::Up(MouseButton::Left) => (0, MouseAction::Release),
        MouseEventKind::Up(MouseButton::Middle) => (1, MouseAction::Release),
        MouseEventKind::Up(MouseButton::Right) => (2, MouseAction::Release),
        MouseEventKind::Drag(MouseButton::Left) => (32, MouseAction::Motion),
        MouseEventKind::Drag(MouseButton::Middle) => (33, MouseAction::Motion),
        MouseEventKind::Drag(MouseButton::Right) => (34, MouseAction::Motion),
        MouseEventKind::Moved => (35, MouseAction::Motion),
        MouseEventKind::ScrollUp => (64, MouseAction::Wheel),
        MouseEventKind::ScrollDown => (65, MouseAction::Wheel),
        MouseEventKind::ScrollLeft => (66, MouseAction::Wheel),
        MouseEventKind::ScrollRight => (67, MouseAction::Wheel),
    })
}

fn mode_allows(mode: MouseProtocolMode, action: MouseAction, button: u8) -> bool {
    match (mode, action) {
        (MouseProtocolMode::None, _) => false,
        (MouseProtocolMode::Press, MouseAction::Press | MouseAction::Wheel) => true,
        (
            MouseProtocolMode::PressRelease,
            MouseAction::Press | MouseAction::Release | MouseAction::Wheel,
        ) => true,
        // Button-event tracking includes drags (buttons 32..=34) but not bare motion (35).
        (MouseProtocolMode::ButtonMotion, MouseAction::Motion) => button != 35,
        (MouseProtocolMode::ButtonMotion, _) => true,
        (MouseProtocolMode::AnyMotion, _) => true,
        _ => false,
    }
}

fn sgr_suffix(action: MouseAction) -> char {
    if matches!(action, MouseAction::Release) {
        'm'
    } else {
        'M'
    }
}

fn modifier_bits(modifiers: KeyModifiers) -> u8 {
    let mut bits = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        bits += 4;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        bits += 8;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        bits += 16;
    }
    bits
}

fn encode_legacy_mouse(
    button: u8,
    x: u16,
    y: u16,
    encoding: MouseProtocolEncoding,
) -> Option<Vec<u8>> {
    let button = button.checked_add(32)?;
    let x_code = x.checked_add(32)?;
    let y_code = y.checked_add(32)?;
    let mut out = vec![0x1b, b'[', b'M'];
    match encoding {
        MouseProtocolEncoding::Default => {
            if x_code > 255 || y_code > 255 {
                return None;
            }
            out.push(button);
            out.push(x_code as u8);
            out.push(y_code as u8);
        }
        MouseProtocolEncoding::Utf8 => {
            push_utf8(&mut out, button as u32);
            push_utf8(&mut out, x_code as u32);
            push_utf8(&mut out, y_code as u32);
        }
        MouseProtocolEncoding::Sgr => return None,
    }
    Some(out)
}

fn push_utf8(out: &mut Vec<u8>, value: u32) {
    if let Some(ch) = char::from_u32(value) {
        let mut buf = [0u8; 4];
        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
    }
}

fn encode_char(ch: char, mods: KeyModifiers) -> Option<Vec<u8>> {
    let alt = mods.contains(KeyModifiers::ALT);
    if mods.contains(KeyModifiers::CONTROL) {
        let byte = ctrl_byte(ch)?;
        return Some(prefix_alt(alt, &[byte]));
    }
    let mut encoded = [0u8; 4];
    let bytes = ch.encode_utf8(&mut encoded).as_bytes();
    Some(prefix_alt(alt, bytes))
}

fn ctrl_byte(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        'A'..='Z' => Some((ch as u8).to_ascii_lowercase() - b'a' + 1),
        ' ' | '@' => Some(0),
        // Crossterm decodes 0x1c..=0x1f as Ctrl+4 through Ctrl+7.
        '4'..='7' => Some(ch as u8 - b'4' + 0x1c),
        '[' => Some(0x1b),
        '\\' => Some(0x1c),
        ']' => Some(0x1d),
        '^' => Some(0x1e),
        '_' => Some(0x1f),
        '?' => Some(0x7f),
        _ => None,
    }
}

fn prefix_alt(alt: bool, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() + usize::from(alt));
    if alt {
        out.push(0x1b);
    }
    out.extend_from_slice(bytes);
    out
}

fn xterm_modifier(mods: KeyModifiers) -> Option<u8> {
    let mut value = 1u8;
    if mods.contains(KeyModifiers::SHIFT) {
        value += 1;
    }
    if mods.contains(KeyModifiers::ALT) {
        value += 2;
    }
    if mods.contains(KeyModifiers::CONTROL) {
        value += 4;
    }
    (value > 1).then_some(value)
}

fn encode_arrow(letter: u8, mods: KeyModifiers, application_cursor: bool) -> Vec<u8> {
    if let Some(modifier) = xterm_modifier(mods) {
        format!("\x1b[1;{modifier}{}", letter as char).into_bytes()
    } else if application_cursor {
        vec![0x1b, b'O', letter]
    } else {
        vec![0x1b, b'[', letter]
    }
}

fn encode_home_end(letter: u8, mods: KeyModifiers, application_cursor: bool) -> Vec<u8> {
    if let Some(modifier) = xterm_modifier(mods) {
        format!("\x1b[1;{modifier}{}", letter as char).into_bytes()
    } else if application_cursor {
        vec![0x1b, b'O', letter]
    } else {
        vec![0x1b, b'[', letter]
    }
}

fn encode_tilde(number: u8, mods: KeyModifiers) -> Vec<u8> {
    if let Some(modifier) = xterm_modifier(mods) {
        format!("\x1b[{number};{modifier}~").into_bytes()
    } else {
        format!("\x1b[{number}~").into_bytes()
    }
}

fn encode_function(n: u8, mods: KeyModifiers) -> Option<Vec<u8>> {
    let kind = match n {
        1 => FuncKind::Letter(b'P'),
        2 => FuncKind::Letter(b'Q'),
        3 => FuncKind::Letter(b'R'),
        4 => FuncKind::Letter(b'S'),
        5 => FuncKind::Tilde(15),
        6 => FuncKind::Tilde(17),
        7 => FuncKind::Tilde(18),
        8 => FuncKind::Tilde(19),
        9 => FuncKind::Tilde(20),
        10 => FuncKind::Tilde(21),
        11 => FuncKind::Tilde(23),
        12 => FuncKind::Tilde(24),
        _ => return None,
    };
    Some(match (kind, xterm_modifier(mods)) {
        (FuncKind::Letter(letter), None) => vec![0x1b, b'O', letter],
        (FuncKind::Letter(letter), Some(modifier)) => {
            format!("\x1b[1;{modifier}{}", letter as char).into_bytes()
        }
        (FuncKind::Tilde(number), None) => format!("\x1b[{number}~").into_bytes(),
        (FuncKind::Tilde(number), Some(modifier)) => {
            format!("\x1b[{number};{modifier}~").into_bytes()
        }
    })
}

enum FuncKind {
    Letter(u8),
    Tilde(u8),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn control_and_arrows_match_xterm() {
        assert_eq!(
            encode_key(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), false).unwrap(),
            vec![0x03]
        );
        assert_eq!(
            encode_key(&key(KeyCode::Char('d'), KeyModifiers::CONTROL), false).unwrap(),
            vec![0x04]
        );
        assert_eq!(
            encode_key(&key(KeyCode::Up, KeyModifiers::NONE), false).unwrap(),
            b"\x1b[A"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Left, KeyModifiers::NONE), true).unwrap(),
            b"\x1bOD"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Left, KeyModifiers::CONTROL), false).unwrap(),
            b"\x1b[1;5D"
        );
        assert_eq!(
            encode_key(&key(KeyCode::Backspace, KeyModifiers::NONE), false).unwrap(),
            vec![0x7f]
        );
    }

    #[test]
    fn control_digits_round_trip_crossterm_control_bytes() {
        for (digit, byte) in [('4', 0x1c), ('5', 0x1d), ('6', 0x1e), ('7', 0x1f)] {
            assert_eq!(
                encode_key(&key(KeyCode::Char(digit), KeyModifiers::CONTROL), false).unwrap(),
                vec![byte]
            );
        }
    }

    #[test]
    fn releases_are_ignored_and_paste_can_be_bracketed() {
        let mut event = key(KeyCode::Char('a'), KeyModifiers::NONE);
        event.kind = KeyEventKind::Release;
        assert!(encode_key(&event, false).is_none());
        assert_eq!(encode_paste("hi", true), b"\x1b[200~hi\x1b[201~");
    }

    #[test]
    fn mouse_modes_follow_the_parser() {
        let mut parser = vt100::Parser::new(24, 80, 0);
        parser.process(b"\x1b[?1000h\x1b[?1006h");
        assert_eq!(
            parser.screen().mouse_protocol_mode(),
            MouseProtocolMode::PressRelease
        );
        assert_eq!(
            parser.screen().mouse_protocol_encoding(),
            MouseProtocolEncoding::Sgr
        );

        let encoded = encode_mouse(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            parser.screen().mouse_protocol_mode(),
            parser.screen().mouse_protocol_encoding(),
        )
        .unwrap();
        assert_eq!(encoded, b"\x1b[<0;1;1M");

        parser.process(b"\x1b[?1000l\x1b[?1006l");
        assert!(encode_mouse(
            MouseEventKind::Down(MouseButton::Left),
            KeyModifiers::NONE,
            0,
            0,
            parser.screen().mouse_protocol_mode(),
            parser.screen().mouse_protocol_encoding(),
        )
        .is_none());
    }
}
