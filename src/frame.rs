use crate::config::{Config, Radius, Rgb, Thickness};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vt100::Screen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentRect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

/// Interior used for the PTY. The border is always one cell thick.
/// Thickness changes the glyphs, not the geometry, so a thicker stroke
/// does not shrink the shell.
pub fn content_rect(term_width: u16, term_height: u16) -> Option<ContentRect> {
    if term_width < 3 || term_height < 3 {
        return None;
    }
    Some(ContentRect {
        x: 1,
        y: 1,
        width: term_width - 2,
        height: term_height - 2,
    })
}

#[derive(Clone, Copy)]
struct Glyphs {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

fn glyphs(radius: Radius, thickness: Thickness) -> Glyphs {
    match (radius, thickness) {
        (Radius::Rounded, Thickness::One) => Glyphs {
            top_left: '╭',
            top_right: '╮',
            bottom_left: '╰',
            bottom_right: '╯',
            horizontal: '─',
            vertical: '│',
        },
        (Radius::Square, Thickness::One) => Glyphs {
            top_left: '+',
            top_right: '+',
            bottom_left: '+',
            bottom_right: '+',
            horizontal: '-',
            vertical: '|',
        },
        (Radius::Rounded, Thickness::Two) => Glyphs {
            top_left: '╭',
            top_right: '╮',
            bottom_left: '╰',
            bottom_right: '╯',
            horizontal: '━',
            vertical: '┃',
        },
        (Radius::Square, Thickness::Two) => Glyphs {
            top_left: '╔',
            top_right: '╗',
            bottom_left: '╚',
            bottom_right: '╝',
            horizontal: '═',
            vertical: '║',
        },
    }
}

pub fn draw(frame: &mut Frame, config: &Config, screen: &Screen) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    let style = Style::default().fg(ratatui_color(config.color));

    let Some(content) = content_rect(area.width, area.height) else {
        paint_message(buf, area, "terminal too small", style);
        return;
    };

    paint_screen(buf, content, screen);
    paint_border(buf, area, config, style);

    if !screen.hide_cursor() {
        let (row, col) = screen.cursor_position();
        if row < content.height && col < content.width {
            frame.set_cursor_position((content.x + col, content.y + row));
        }
    }
}

fn paint_message(buf: &mut ratatui::buffer::Buffer, area: Rect, message: &str, style: Style) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let width = UnicodeWidthStr::width(message) as u16;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height / 2;
    if y < area.y.saturating_add(area.height) {
        buf.set_string(x, y, truncate_width(message, area.width as usize), style);
    }
}

fn paint_border(buf: &mut ratatui::buffer::Buffer, area: Rect, config: &Config, style: Style) {
    let glyphs = glyphs(config.radius, config.thickness);
    let width = area.width;
    let height = area.height;
    if width == 0 || height == 0 {
        return;
    }

    put(buf, area.x, area.y, glyphs.top_left, style);
    if width > 1 {
        put(buf, area.right() - 1, area.y, glyphs.top_right, style);
    }
    if height > 1 {
        put(buf, area.x, area.bottom() - 1, glyphs.bottom_left, style);
        if width > 1 {
            put(
                buf,
                area.right() - 1,
                area.bottom() - 1,
                glyphs.bottom_right,
                style,
            );
        }
    }

    if width > 2 {
        for col in 1..width - 1 {
            put(buf, area.x + col, area.y, glyphs.horizontal, style);
            if height > 1 {
                put(
                    buf,
                    area.x + col,
                    area.bottom() - 1,
                    glyphs.horizontal,
                    style,
                );
            }
        }
    }
    if height > 2 {
        for row in 1..height - 1 {
            put(buf, area.x, area.y + row, glyphs.vertical, style);
            if width > 1 {
                put(buf, area.right() - 1, area.y + row, glyphs.vertical, style);
            }
        }
    }

    paint_brand(buf, area, &config.brand, glyphs.horizontal, style);
}

fn paint_brand(
    buf: &mut ratatui::buffer::Buffer,
    area: Rect,
    brand: &str,
    _horizontal: char,
    style: Style,
) {
    if brand.is_empty() || area.width < 6 {
        return;
    }
    let max_text = area.width as usize - 5;
    let text = truncate_width(brand, max_text);
    let text_width = UnicodeWidthStr::width(text);
    if text_width == 0 {
        return;
    }
    let start = area.x + 3;
    put(buf, area.x + 2, area.y, ' ', style);
    buf.set_string(start, area.y, text, style);
    let after = start + text_width as u16;
    if after < area.right() - 1 {
        put(buf, after, area.y, ' ', style);
    }
}

fn paint_screen(buf: &mut ratatui::buffer::Buffer, content: ContentRect, screen: &Screen) {
    let (rows, cols) = screen.size();
    let rows = rows.min(content.height);
    let cols = cols.min(content.width);
    for row in 0..rows {
        let mut col = 0u16;
        while col < cols {
            let Some(cell) = screen.cell(row, col) else {
                break;
            };
            if cell.is_wide_continuation() {
                col += 1;
                continue;
            }
            let span = if cell.is_wide() { 2 } else { 1 };
            if col + span > content.width {
                break;
            }
            let style = cell_style(cell);
            let x = content.x + col;
            let y = content.y + row;
            if cell.has_contents() {
                buf.set_string(x, y, cell.contents(), style);
            } else {
                buf.set_string(x, y, " ", style);
            }
            col += span;
        }
    }
}

fn cell_style(cell: &vt100::Cell) -> Style {
    let mut style = Style::default()
        .fg(vt_color(cell.fgcolor()))
        .bg(vt_color(cell.bgcolor()));
    let mut modifiers = Modifier::empty();
    if cell.bold() {
        modifiers |= Modifier::BOLD;
    }
    if cell.italic() {
        modifiers |= Modifier::ITALIC;
    }
    if cell.underline() {
        modifiers |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        modifiers |= Modifier::REVERSED;
    }
    style = style.add_modifier(modifiers);
    style
}

fn vt_color(color: vt100::Color) -> Color {
    match color {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(index) => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn ratatui_color(color: Rgb) -> Color {
    Color::Rgb(color.r, color.g, color.b)
}

fn put(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, ch: char, style: Style) {
    if x >= buf.area.width || y >= buf.area.height {
        return;
    }
    buf[(x, y)].set_char(ch).set_style(style);
}

fn truncate_width(text: &str, max: usize) -> &str {
    let mut width = 0usize;
    for (index, ch) in text.char_indices() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + char_width > max {
            return &text[..index];
        }
        width += char_width;
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(toml: &str, columns: u16, rows: u16, input: &[u8]) -> ratatui::buffer::Buffer {
        let config = parse_config(toml).unwrap();
        let content = content_rect(columns, rows).unwrap();
        let mut parser = vt100::Parser::new(content.height, content.width, 0);
        parser.process(input);
        let backend = TestBackend::new(columns, rows);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, &config, parser.screen()))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn line(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect()
    }

    #[test]
    fn rounded_frame_keeps_brand_and_clips_shell_to_the_interior() {
        let buf = render(crate::config::DEFAULT_CONFIG, 24, 6, b"\x1b[31mhello");
        let top = line(&buf, 0);
        assert!(top.starts_with("╭─ BOXXY "), "{top}");
        assert!(top.ends_with('╮'), "{top}");
        assert_eq!(buf[(0, 1)].symbol(), "│");
        assert_eq!(buf[(23, 1)].symbol(), "│");
        assert!(line(&buf, 5).starts_with('╰') && line(&buf, 5).ends_with('╯'));
        assert_eq!(buf[(1, 1)].symbol(), "h");
        assert_eq!(buf[(5, 1)].symbol(), "o");
        assert_eq!(buf[(0, 0)].style().fg, Some(Color::Rgb(0x89, 0xDC, 0xEB)));
    }

    #[test]
    fn square_and_thick_borders_use_their_glyphs() {
        let square = "\
[frame]
thickness = 1
radius = \"square\"
color = \"#89DCEB\"

[brand]
text = \"LAB\"
";
        let buf = render(square, 16, 5, b"");
        assert_eq!(buf[(0, 0)].symbol(), "+");
        assert_eq!(buf[(0, 1)].symbol(), "|");
        assert!(line(&buf, 0).contains("LAB"));

        let thick = "\
[frame]
thickness = 2
radius = \"square\"
color = \"#F38BA8\"

[brand]
text = \"LAB\"
";
        let buf = render(thick, 16, 5, b"\x1b[2J\x1b[H");
        assert_eq!(buf[(0, 0)].symbol(), "╔");
        assert_eq!(buf[(15, 0)].symbol(), "╗");
        assert_eq!(buf[(0, 1)].symbol(), "║");
        assert_eq!(buf[(0, 4)].symbol(), "╚");
        assert_eq!(buf[(15, 4)].symbol(), "╝");
    }

    #[test]
    fn content_rect_rejects_a_terminal_smaller_than_the_frame() {
        assert!(content_rect(2, 10).is_none());
        assert!(content_rect(10, 2).is_none());
        assert_eq!(
            content_rect(80, 24),
            Some(ContentRect {
                x: 1,
                y: 1,
                width: 78,
                height: 22
            })
        );
    }
}
