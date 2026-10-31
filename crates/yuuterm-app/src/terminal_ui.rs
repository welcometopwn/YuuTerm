use eframe::egui::{self, Color32, FontId};
use yuuterm_ssh::{
    Command, LiveSession,
    terminal::{Color, TerminalView},
};

pub struct PendingPaste {
    pub live_id: u64,
    pub text: String,
}

#[derive(Default)]
pub struct RenderState {
    size: Option<(u16, u16)>,
    pub clicked: bool,
}

pub fn paste(session: &LiveSession, view: &TerminalView, text: &str) -> Result<(), &'static str> {
    if text.len() > 16000 {
        return Err("Paste exceeds the 16000-byte preview limit.");
    }
    let bytes = if view.bracketed_paste {
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.as_bytes().to_vec()
    };
    session.send(Command::Input(bytes))
}

pub fn show(
    ui: &mut egui::Ui,
    session: &LiveSession,
    font_size: f32,
    render: &mut RenderState,
    notice: &mut String,
) -> Option<PendingPaste> {
    let snapshot = session.snapshot();
    let view = &snapshot.terminal;
    let font = FontId::monospace(font_size);
    let char_width = ui.fonts_mut(|f| f.glyph_width(&font, 'M'));
    let line_height = font_size * 1.3;
    let (rect, response) = ui.allocate_exact_size(
        ui.available_size().max(egui::vec2(80.0, 40.0)),
        egui::Sense::click_and_drag(),
    );
    render.clicked = response.clicked();
    if response.clicked() {
        response.request_focus();
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, Color32::from_rgb(15, 16, 19));
    let (rows, cols) = yuuterm_ssh::terminal::dimensions(
        (rect.height() / line_height) as u16,
        (rect.width() / char_width) as u16,
    );
    if snapshot.connected
        && render.size != Some((rows, cols))
        && session.send(Command::Resize { rows, cols }).is_ok()
    {
        render.size = Some((rows, cols));
    }
    for row in 0..view.rows {
        for col in 0..view.cols {
            let cell = &view.cells[(row as usize) * (view.cols as usize) + col as usize];
            let pos = rect.min + egui::vec2(col as f32 * char_width, row as f32 * line_height);
            if !rect.contains(pos) {
                continue;
            }
            let mut fg = color(cell.fg, Color32::from_rgb(220, 223, 230));
            let mut bg = color(cell.bg, Color32::from_rgb(15, 16, 19));
            if cell.inverse {
                std::mem::swap(&mut fg, &mut bg);
            }
            let cell_rect = egui::Rect::from_min_size(pos, egui::vec2(char_width, line_height));
            if bg != Color32::from_rgb(15, 16, 19) {
                painter.rect_filled(cell_rect, 0, bg);
            }
            if !cell.wide_continuation && !cell.text.is_empty() {
                painter.text(pos, egui::Align2::LEFT_TOP, &cell.text, font.clone(), fg);
                if cell.underline {
                    painter.line_segment(
                        [cell_rect.left_bottom(), cell_rect.right_bottom()],
                        (1.0, fg),
                    );
                }
            }
        }
    }
    if response.has_focus() && !view.hide_cursor {
        let (row, col) = view.cursor;
        let pos = rect.min + egui::vec2(col as f32 * char_width, row as f32 * line_height);
        painter.rect_stroke(
            egui::Rect::from_min_size(pos, egui::vec2(char_width, line_height)),
            0,
            (1.0, Color32::LIGHT_GRAY),
            egui::StrokeKind::Inside,
        );
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.1 {
            let _ = session.send(Command::Scroll(if scroll > 0.0 { 3 } else { -3 }));
        }
    }
    if !ui.is_enabled() || !response.has_focus() || !snapshot.connected {
        return None;
    }
    let mut pending = None;
    for event in ui.input(|i| i.events.clone()) {
        let bytes = match event {
            egui::Event::Text(text) => Some(text.into_bytes()),
            egui::Event::Paste(text) => {
                if text.len() > 16000 {
                    *notice = "Paste exceeds the 16000-byte limit; nothing was sent.".into();
                } else if text.chars().any(char::is_control) {
                    pending = Some(PendingPaste {
                        live_id: session.id,
                        text,
                    });
                } else if let Err(e) = paste(session, view, &text) {
                    *notice = e.into();
                }
                None
            }
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => key_bytes(key, modifiers, view.application_cursor),
            _ => None,
        };
        if let Some(bytes) = bytes
            && let Err(e) = session.send(Command::Input(bytes))
        {
            *notice = e.into();
        }
    }
    pending
}

fn key_bytes(key: egui::Key, mods: egui::Modifiers, application: bool) -> Option<Vec<u8>> {
    use egui::Key::*;
    if mods.ctrl && !mods.alt && !mods.shift {
        let c = match key {
            A => 1,
            B => 2,
            C => 3,
            D => 4,
            E => 5,
            F => 6,
            G => 7,
            H => 8,
            I => 9,
            J => 10,
            K => 11,
            L => 12,
            M => 13,
            N => 14,
            O => 15,
            P => 16,
            Q => 17,
            R => 18,
            S => 19,
            T => 20,
            U => 21,
            W => 23,
            X => 24,
            Y => 25,
            Z => 26,
            _ => return None,
        };
        return Some(vec![c]);
    }
    // AltGr text arrives as Event::Text; never turn it into Ctrl sequences.
    if mods.ctrl && mods.alt {
        return None;
    }
    let seq = match key {
        Enter => "\r",
        Backspace => "\x7f",
        Escape => "\x1b",
        Tab if mods.shift => "\x1b[Z",
        Tab => "\t",
        ArrowUp if application => "\x1bOA",
        ArrowDown if application => "\x1bOB",
        ArrowRight if application => "\x1bOC",
        ArrowLeft if application => "\x1bOD",
        ArrowUp => "\x1b[A",
        ArrowDown => "\x1b[B",
        ArrowRight => "\x1b[C",
        ArrowLeft => "\x1b[D",
        Home => "\x1b[H",
        End => "\x1b[F",
        Delete => "\x1b[3~",
        Insert => "\x1b[2~",
        PageUp => "\x1b[5~",
        PageDown => "\x1b[6~",
        F1 => "\x1bOP",
        F2 => "\x1bOQ",
        F3 => "\x1bOR",
        F4 => "\x1bOS",
        F5 => "\x1b[15~",
        F6 => "\x1b[17~",
        F7 => "\x1b[18~",
        F8 => "\x1b[19~",
        F9 => "\x1b[20~",
        F10 => "\x1b[21~",
        F11 => "\x1b[23~",
        F12 => "\x1b[24~",
        _ => return None,
    };
    Some(seq.as_bytes().to_vec())
}

fn color(color: Color, default: Color32) -> Color32 {
    match color {
        Color::Default => default,
        Color::Rgb(r, g, b) => Color32::from_rgb(r, g, b),
        Color::Idx(n) => {
            const ANSI: [[u8; 3]; 16] = [
                [0, 0, 0],
                [205, 49, 49],
                [13, 188, 121],
                [229, 229, 16],
                [36, 114, 200],
                [188, 63, 188],
                [17, 168, 205],
                [229, 229, 229],
                [102, 102, 102],
                [241, 76, 76],
                [35, 209, 139],
                [245, 245, 67],
                [59, 142, 234],
                [214, 112, 214],
                [41, 184, 219],
                [255, 255, 255],
            ];
            let rgb = if n < 16 {
                ANSI[n as usize]
            } else if n < 232 {
                let n = n - 16;
                let ramp = |x: u8| if x == 0 { 0 } else { 55 + 40 * x };
                [ramp(n / 36), ramp((n / 6) % 6), ramp(n % 6)]
            } else {
                [8 + 10 * (n - 232); 3]
            };
            Color32::from_rgb(rgb[0], rgb[1], rgb[2])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ctrl_c_arrows_and_altgr_are_distinct() {
        assert_eq!(
            key_bytes(egui::Key::C, egui::Modifiers::CTRL, false),
            Some(vec![3])
        );
        assert_eq!(
            key_bytes(egui::Key::ArrowUp, egui::Modifiers::NONE, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes(
                egui::Key::Q,
                egui::Modifiers {
                    ctrl: true,
                    alt: true,
                    ..Default::default()
                },
                false
            ),
            None
        );
    }
}
