use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Cell {
    pub text: String,
    pub fg: vt100::Color,
    pub bg: vt100::Color,
    pub bold: bool,
    pub inverse: bool,
    pub underline: bool,
    pub wide_continuation: bool,
}

#[derive(Clone, Debug)]
pub struct TerminalView {
    pub rows: u16,
    pub cols: u16,
    pub cells: Vec<Cell>,
    pub cursor: (u16, u16),
    pub hide_cursor: bool,
    pub application_cursor: bool,
    pub bracketed_paste: bool,
}

pub(crate) struct Terminal {
    parser: vt100::Parser,
}

pub fn dimensions(rows: u16, cols: u16) -> (u16, u16) {
    (rows.clamp(2, 100), cols.clamp(10, 240))
}

impl Terminal {
    pub fn new() -> Self {
        Self {
            parser: vt100::Parser::new(24, 80, 500),
        }
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }
    pub fn resize(&mut self, rows: u16, cols: u16) {
        let (rows, cols) = dimensions(rows, cols);
        self.parser.screen_mut().set_size(rows, cols);
    }
    pub fn scroll(&mut self, delta: i32) {
        let current = self.parser.screen().scrollback();
        self.parser
            .screen_mut()
            .set_scrollback(current.saturating_add_signed(delta as isize).min(500));
    }
    pub fn view(&self) -> Arc<TerminalView> {
        let screen = self.parser.screen();
        let (rows, cols) = screen.size();
        let cells = (0..rows)
            .flat_map(|row| {
                (0..cols).map(move |col| {
                    let c = screen.cell(row, col).expect("within screen bounds");
                    Cell {
                        text: c.contents().to_owned(),
                        fg: c.fgcolor(),
                        bg: c.bgcolor(),
                        bold: c.bold(),
                        inverse: c.inverse(),
                        underline: c.underline(),
                        wide_continuation: c.is_wide_continuation(),
                    }
                })
            })
            .collect();
        Arc::new(TerminalView {
            rows,
            cols,
            cells,
            cursor: screen.cursor_position(),
            hide_cursor: screen.hide_cursor(),
            application_cursor: screen.application_cursor(),
            bracketed_paste: screen.bracketed_paste(),
        })
    }
}

pub use vt100::Color;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fragmented_utf8_colors_cursor_and_resize() {
        let mut t = Terminal::new();
        t.process(b"\x1b[31mGr\xc3");
        t.process(b"\xbc\xc3\x9fe\x1b[0m");
        let v = t.view();
        assert_eq!(v.cells[2].text, "ü");
        assert_eq!(v.cells[0].fg, Color::Idx(1));
        t.process(b"\x1b[2J\x1b[Hok");
        assert_eq!(t.view().cells[0].text, "o");
        t.resize(65535, 65535);
        assert_eq!((t.view().rows, t.view().cols), (100, 240));
        t.process(b"\x1b]52;c;aGVsbG8=\x07"); // ignored; no clipboard callback installed
    }
}
