use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{cursor, execute};
use portable_pty::{native_pty_system, Child, CommandBuilder, PtySize};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::config::Config;
use crate::frame::{self, content_rect, ContentRect};
use crate::keys::{self, encode_key};

#[derive(Debug)]
pub struct SessionError {
    message: String,
}

impl SessionError {
    fn detail(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error:\n{}", self.message)
    }
}

impl std::error::Error for SessionError {}

impl From<io::Error> for SessionError {
    fn from(err: io::Error) -> Self {
        Self::detail(format!("terminal error: {err}"))
    }
}

/// Restore the user's terminal even when the session panics or returns early.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let mut stdout = io::stdout();
        let _ = execute!(
            stdout,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen,
            cursor::Show,
        );
    }
}

enum PtyMsg {
    Data(Vec<u8>),
    Closed,
}

pub fn run(config: &Config) -> Result<i32, SessionError> {
    let (cols, rows) = crossterm::terminal::size()?;
    let Some(content) = content_rect(cols, rows) else {
        return Err(SessionError::detail(format!(
            "Terminal is too small for the frame (got {cols}x{rows})."
        )));
    };

    // Take the terminal only after the frame is known to fit. The guard runs
    // on every exit path, including a panic inside the event loop.
    enable_raw_mode()?;
    let _guard = TerminalGuard;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let shell = shell_program();
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(pty_size(content))
        .map_err(|err| SessionError::detail(format!("could not open a pty: {err}")))?;

    let mut command = CommandBuilder::new(shell.as_os_str());
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("LINES", content.height.to_string());
    command.env("COLUMNS", content.width.to_string());

    let slave = pair.slave;
    let master = pair.master;
    let mut child = slave.spawn_command(command).map_err(|err| {
        SessionError::detail(format!("could not start {}: {err}", shell.display()))
    })?;
    // The parent must not keep the slave open, or the master never reaches EOF
    // when the shell exits and boxxy would stay up.
    drop(slave);
    let reader = master
        .try_clone_reader()
        .map_err(|err| SessionError::detail(format!("could not read the shell: {err}")))?;
    let mut writer = master
        .take_writer()
        .map_err(|err| SessionError::detail(format!("could not write to the shell: {err}")))?;

    let rx = spawn_reader(reader);
    let mut parser = vt100::Parser::new(content.height, content.width, 0);
    let mut content = content;
    let mut bracketed_paste = false;
    let mut needs_draw = true;

    loop {
        if crossterm::event::poll(Duration::from_millis(10))? {
            while crossterm::event::poll(Duration::ZERO)? {
                match crossterm::event::read()? {
                    Event::Key(key) => {
                        if let Some(bytes) = encode_key(&key, parser.screen().application_cursor())
                        {
                            if write_pty(&mut writer, &bytes).is_err() {
                                return finish_child(&mut *child);
                            }
                        }
                    }
                    Event::Paste(text) => {
                        let bytes = keys::encode_paste(&text, parser.screen().bracketed_paste());
                        if write_pty(&mut writer, &bytes).is_err() {
                            return finish_child(&mut *child);
                        }
                    }
                    Event::Mouse(mouse) => {
                        if let Some(local) = content_coords(content, mouse.column, mouse.row) {
                            if let Some(bytes) = keys::encode_mouse(
                                mouse.kind,
                                mouse.modifiers,
                                local.0,
                                local.1,
                                parser.screen().mouse_protocol_mode(),
                                parser.screen().mouse_protocol_encoding(),
                            ) {
                                if write_pty(&mut writer, &bytes).is_err() {
                                    return finish_child(&mut *child);
                                }
                            }
                        }
                    }
                    Event::Resize(cols, rows) => {
                        if let Some(next) = content_rect(cols, rows) {
                            if next != content {
                                parser.set_size(next.height, next.width);
                                master.resize(pty_size(next)).map_err(|err| {
                                    SessionError::detail(format!(
                                        "could not resize the shell: {err}"
                                    ))
                                })?;
                                content = next;
                            }
                        }
                        needs_draw = true;
                    }
                    Event::FocusGained | Event::FocusLost => {}
                }
            }
        }

        let mut saw_output = false;
        let mut closed = false;
        while let Ok(message) = rx.try_recv() {
            match message {
                PtyMsg::Data(bytes) => {
                    parser.process(&bytes);
                    saw_output = true;
                }
                PtyMsg::Closed => closed = true,
            }
        }

        if saw_output {
            let want_bracketed = parser.screen().bracketed_paste();
            if want_bracketed != bracketed_paste {
                set_bracketed_paste(want_bracketed)?;
                bracketed_paste = want_bracketed;
            }
        }

        if saw_output || needs_draw {
            terminal.draw(|ui| frame::draw(ui, config, parser.screen()))?;
            needs_draw = false;
        }

        if closed {
            break;
        }
    }

    finish_child(&mut *child)
}

fn content_coords(content: ContentRect, column: u16, row: u16) -> Option<(u16, u16)> {
    if column < content.x || row < content.y {
        return None;
    }
    let local_col = column - content.x;
    let local_row = row - content.y;
    if local_col >= content.width || local_row >= content.height {
        return None;
    }
    Some((local_col, local_row))
}

fn pty_size(content: ContentRect) -> PtySize {
    PtySize {
        rows: content.height,
        cols: content.width,
        pixel_width: 0,
        pixel_height: 0,
    }
}

fn shell_program() -> PathBuf {
    if let Some(shell) = std::env::var_os("SHELL") {
        if !shell.is_empty() && PathBuf::from(&shell).is_file() {
            return PathBuf::from(shell);
        }
    }
    PathBuf::from("/bin/sh")
}

fn spawn_reader(mut reader: Box<dyn Read + Send>) -> std::sync::mpsc::Receiver<PtyMsg> {
    let (tx, rx) = std::sync::mpsc::sync_channel(64);
    std::thread::Builder::new()
        .name("boxxy-pty".into())
        .spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        let _ = tx.send(PtyMsg::Closed);
                        break;
                    }
                    Ok(count) => {
                        if tx.send(PtyMsg::Data(buf[..count].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = tx.send(PtyMsg::Closed);
                        break;
                    }
                }
            }
        })
        .expect("boxxy pty reader thread");
    rx
}

fn write_pty(writer: &mut dyn Write, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}

fn set_bracketed_paste(enabled: bool) -> io::Result<()> {
    let mut stdout = io::stdout();
    if enabled {
        execute!(stdout, EnableBracketedPaste)?;
    } else {
        execute!(stdout, DisableBracketedPaste)?;
    }
    Ok(())
}

fn finish_child(child: &mut dyn Child) -> Result<i32, SessionError> {
    match child.try_wait() {
        Ok(Some(status)) => Ok(exit_code_of(&status)),
        Ok(None) => {
            let _ = child.kill();
            child
                .wait()
                .map(|status| exit_code_of(&status))
                .map_err(|err| SessionError::detail(format!("could not wait for the shell: {err}")))
        }
        Err(err) => Err(SessionError::detail(format!(
            "could not wait for the shell: {err}"
        ))),
    }
}

fn exit_code_of(status: &portable_pty::ExitStatus) -> i32 {
    if let Some(signal) = status.signal() {
        if let Some(number) = signal_number(signal) {
            return 128 + number;
        }
        return 1;
    }
    let code = status.exit_code();
    if code > 255 {
        1
    } else {
        code as i32
    }
}

fn signal_number(signal: &str) -> Option<i32> {
    let name = signal
        .rsplit(|ch: char| !ch.is_ascii_alphabetic())
        .next()
        .unwrap_or(signal);
    let name = name.strip_prefix("SIG").unwrap_or(name);
    Some(match name {
        "HUP" => 1,
        "INT" => 2,
        "QUIT" => 3,
        "ILL" => 4,
        "TRAP" => 5,
        "ABRT" | "IOT" => 6,
        "BUS" => 7,
        "FPE" => 8,
        "KILL" => 9,
        "USR1" => 10,
        "SEGV" => 11,
        "USR2" => 12,
        "PIPE" => 13,
        "ALRM" => 14,
        "TERM" => 15,
        _ => return None,
    })
}
