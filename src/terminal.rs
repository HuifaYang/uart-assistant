use crate::theme::{Rgb, TermPalette};
use crate::types::Segment;
use std::collections::VecDeque;

/// Attr 只保存颜色"槽位"：0-15 为 ANSI，16 为默认前景，17+ 为 xterm-256 色。
/// 具体 RGB 在渲染时按当前主题解析，因此切换主题后已有内容也会跟着变色。
pub const DEFAULT_COLOR: u16 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub color: u16,
    pub bold: bool,
    pub inverse: bool,
}

impl Default for Attr {
    fn default() -> Self {
        Self {
            color: DEFAULT_COLOR,
            bold: false,
            inverse: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cell {
    pub ch: char,
    pub attr: Attr,
}

impl Cell {
    fn blank() -> Self {
        Self {
            ch: ' ',
            attr: Attr::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseState {
    Ground,
    Escape,
    Csi,
    Osc,
    OscEscape,
}

pub struct TerminalEmulator {
    pub cols: usize,
    pub rows: usize,
    screen: Vec<Vec<Cell>>,
    alt: Vec<Vec<Cell>>,
    using_alt: bool,
    scrollback: VecDeque<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    saved: (usize, usize),
    attr: Attr,
    state: ParseState,
    params: Vec<u32>,
    private: bool,
    osc: String,
}

impl TerminalEmulator {
    pub fn new(cols: usize, rows: usize) -> Self {
        let blank = vec![Cell::blank(); cols];
        Self {
            cols,
            rows,
            screen: vec![blank.clone(); rows],
            alt: vec![blank; rows],
            using_alt: false,
            scrollback: VecDeque::new(),
            cursor_row: 0,
            cursor_col: 0,
            saved: (0, 0),
            attr: Attr::default(),
            state: ParseState::Ground,
            params: Vec::new(),
            private: false,
            osc: String::new(),
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.clamp(20, 300);
        let rows = rows.clamp(4, 100);
        if cols == self.cols && rows == self.rows {
            return;
        }
        let blank = vec![Cell::blank(); cols];
        self.screen.resize(rows, blank.clone());
        self.alt.resize(rows, blank);
        for row in &mut self.screen {
            row.resize(cols, Cell::blank());
        }
        for row in &mut self.alt {
            row.resize(cols, Cell::blank());
        }
        self.cols = cols;
        self.rows = rows;
        self.cursor_row = self.cursor_row.min(rows - 1);
        self.cursor_col = self.cursor_col.min(cols - 1);
    }

    fn blank_row(&self) -> Vec<Cell> {
        vec![Cell::blank(); self.cols]
    }

    fn line_feed(&mut self) {
        if self.cursor_row + 1 < self.rows {
            self.cursor_row += 1;
        } else {
            let top = self.screen.remove(0);
            if !self.using_alt {
                self.scrollback.push_back(top);
                while self.scrollback.len() > 2000 {
                    self.scrollback.pop_front();
                }
            }
            self.screen.push(self.blank_row());
        }
    }

    fn put_char(&mut self, ch: char) {
        if self.cursor_col >= self.cols {
            self.cursor_col = 0;
            self.line_feed();
        }
        let attr = self.attr;
        if self.cursor_row < self.screen.len() && self.cursor_col < self.screen[self.cursor_row].len() {
            self.screen[self.cursor_row][self.cursor_col] = Cell { ch, attr };
        }
        self.cursor_col += 1;
    }

    fn ground(&mut self, ch: char) {
        match ch {
            '\x1b' => self.state = ParseState::Escape,
            '\r' => self.cursor_col = 0,
            '\n' => self.line_feed(),
            '\x08' => self.cursor_col = self.cursor_col.saturating_sub(1),
            '\t' => {
                let next = (self.cursor_col / 8 + 1) * 8;
                self.cursor_col = next.min(self.cols.saturating_sub(1));
            }
            '\x07' | '\x00' => {}
            c if c < ' ' => {}
            c => self.put_char(c),
        }
    }

    pub fn feed(&mut self, text: &str) {
        for ch in text.chars() {
            match self.state {
                ParseState::Ground => self.ground(ch),
                ParseState::Escape => {
                    match ch {
                        '[' => {
                            self.state = ParseState::Csi;
                            self.params.clear();
                            self.private = false;
                        }
                        ']' => {
                            self.state = ParseState::Osc;
                            self.osc.clear();
                        }
                        '7' => self.saved = (self.cursor_row, self.cursor_col),
                        '8' => {
                            let (r, c) = self.saved;
                            self.cursor_row = r.min(self.rows - 1);
                            self.cursor_col = c.min(self.cols - 1);
                        }
                        'D' => self.line_feed(),
                        'E' => {
                            self.cursor_col = 0;
                            self.line_feed();
                        }
                        'M' => {
                            if self.cursor_row > 0 {
                                self.cursor_row -= 1;
                            } else {
                                self.screen.pop();
                                self.screen.insert(0, self.blank_row());
                            }
                        }
                        'c' => {
                            let cols = self.cols;
                            let rows = self.rows;
                            *self = Self::new(cols, rows);
                        }
                        _ => {}
                    }
                    if self.state == ParseState::Escape {
                        self.state = ParseState::Ground;
                    }
                }
                ParseState::Csi => {
                    if ch == '?' {
                        self.private = true;
                    } else if ch.is_ascii_digit() {
                        let idx = self.params.len().saturating_sub(1);
                        if self.params.is_empty() {
                            self.params.push(ch as u32 - '0' as u32);
                        } else {
                            self.params[idx] = self.params[idx]
                                .saturating_mul(10)
                                .saturating_add(ch as u32 - '0' as u32);
                        }
                    } else if ch == ';' {
                        self.params.push(0);
                    } else if ch >= '@' as char && ch <= '~' as char {
                        self.dispatch_csi(ch);
                        self.state = ParseState::Ground;
                    } else {
                        self.state = ParseState::Ground;
                    }
                }
                ParseState::Osc => {
                    if ch == '\x07' {
                        self.state = ParseState::Ground;
                    } else if ch == '\x1b' {
                        self.state = ParseState::OscEscape;
                    } else {
                        self.osc.push(ch);
                    }
                }
                ParseState::OscEscape => {
                    self.state = if ch == '\\' {
                        ParseState::Ground
                    } else {
                        ParseState::Osc
                    };
                }
            }
        }
    }

    fn arg(&self, idx: usize, default: u32) -> u32 {
        let value = self.params.get(idx).copied().unwrap_or(default);
        if value == 0 {
            default
        } else {
            value
        }
    }

    fn dispatch_csi(&mut self, final_ch: char) {
        match final_ch {
            'A' => self.cursor_row = self.cursor_row.saturating_sub(self.arg(0, 1) as usize),
            'B' => {
                self.cursor_row = (self.cursor_row + self.arg(0, 1) as usize).min(self.rows - 1)
            }
            'C' => {
                self.cursor_col = (self.cursor_col + self.arg(0, 1) as usize).min(self.cols - 1)
            }
            'D' => self.cursor_col = self.cursor_col.saturating_sub(self.arg(0, 1) as usize),
            'G' => {
                self.cursor_col = (self.arg(0, 1) as usize - 1).min(self.cols - 1)
            }
            'd' => {
                self.cursor_row = (self.arg(0, 1) as usize - 1).min(self.rows - 1)
            }
            'H' | 'f' => {
                self.cursor_row = (self.arg(0, 1) as usize - 1).min(self.rows - 1);
                self.cursor_col = (self.arg(1, 1) as usize - 1).min(self.cols - 1);
            }
            'J' => {
                let mode = self.params.first().copied().unwrap_or(0);
                match mode {
                    0 => {
                        self.clear_range(self.cursor_col, self.cols, self.cursor_row);
                        for row in self.cursor_row + 1..self.rows {
                            self.screen[row] = self.blank_row();
                        }
                    }
                    1 => {
                        for row in 0..self.cursor_row {
                            self.screen[row] = self.blank_row();
                        }
                        self.clear_range(0, self.cursor_col + 1, self.cursor_row);
                    }
                    _ => {
                        self.screen = vec![self.blank_row(); self.rows];
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                    }
                }
            }
            'K' => {
                let mode = self.params.first().copied().unwrap_or(0);
                match mode {
                    0 => self.clear_range(self.cursor_col, self.cols, self.cursor_row),
                    1 => self.clear_range(0, self.cursor_col + 1, self.cursor_row),
                    _ => self.screen[self.cursor_row] = self.blank_row(),
                }
            }
            'm' => self.apply_sgr(),
            's' => self.saved = (self.cursor_row, self.cursor_col),
            'u' => {
                let (r, c) = self.saved;
                self.cursor_row = r.min(self.rows - 1);
                self.cursor_col = c.min(self.cols - 1);
            }
            'S' => {
                for _ in 0..self.arg(0, 1) {
                    let blank = self.blank_row();
                    self.screen.remove(0);
                    self.screen.push(blank);
                }
            }
            'T' => {
                for _ in 0..self.arg(0, 1) {
                    let blank = self.blank_row();
                    self.screen.pop();
                    self.screen.insert(0, blank);
                }
            }
            'P' => {
                let count = self.arg(0, 1) as usize;
                let row = self.cursor_row;
                let col = self.cursor_col;
                for _ in 0..count {
                    if col < self.screen[row].len() {
                        self.screen[row].remove(col);
                        self.screen[row].push(Cell::blank());
                    }
                }
            }
            '@' => {
                let count = self.arg(0, 1) as usize;
                let row = self.cursor_row;
                let col = self.cursor_col;
                for _ in 0..count {
                    if col < self.screen[row].len() {
                        self.screen[row].insert(col, Cell::blank());
                        self.screen[row].truncate(self.cols);
                    }
                }
            }
            'h' | 'l' if self.private => {
                let mode = self.params.first().copied().unwrap_or(0);
                if mode == 1049 {
                    let enable = final_ch == 'h';
                    if enable != self.using_alt {
                        std::mem::swap(&mut self.screen, &mut self.alt);
                        self.using_alt = enable;
                        if enable {
                            self.alt = vec![self.blank_row(); self.rows];
                        }
                        self.cursor_row = 0;
                        self.cursor_col = 0;
                    }
                }
            }
            _ => {}
        }
    }

    fn clear_range(&mut self, from: usize, to: usize, row: usize) {
        if row >= self.screen.len() {
            return;
        }
        for col in from.min(self.cols)..to.min(self.cols) {
            self.screen[row][col] = Cell::blank();
        }
    }

    fn apply_sgr(&mut self) {
        if self.params.is_empty() {
            self.attr = Attr::default();
            return;
        }
        let mut i = 0usize;
        while i < self.params.len() {
            let code = self.params[i];
            match code {
                0 => self.attr = Attr::default(),
                1 => self.attr.bold = true,
                7 => self.attr.inverse = true,
                22 => self.attr.bold = false,
                27 => self.attr.inverse = false,
                30..=37 => self.attr.color = (code - 30) as u16,
                90..=97 => self.attr.color = 8 + (code - 90) as u16,
                39 => self.attr.color = DEFAULT_COLOR,
                38 if i + 2 < self.params.len() && self.params[i + 1] == 5 => {
                    self.attr.color = 17 + (self.params[i + 2] as u16).min(255);
                    i += 2;
                }
                40..=47 | 100..=107 | 48 | 49 => {}
                _ => {}
            }
            i += 1;
        }
    }

    pub fn render_lines(&self, show_cursor: bool, pal: &TermPalette) -> Vec<Vec<Segment>> {
        self.screen
            .iter()
            .enumerate()
            .map(|(row_index, row)| {
                let mut segments: Vec<Segment> = Vec::new();
                let mut current = String::new();
                let mut style: Option<(Rgb, bool)> = None;
                for (col, cell) in row.iter().enumerate() {
                    let at_cursor = row_index == self.cursor_row && col == self.cursor_col;
                    let ch = if at_cursor { '█' } else { cell.ch };
                    let next = if at_cursor {
                        (if show_cursor { pal.cursor } else { pal.cursor_dim }, true)
                    } else {
                        (
                            resolve(cell.attr.color, cell.attr.inverse, pal),
                            cell.attr.bold,
                        )
                    };
                    match style {
                        Some(existing) if existing != next => {
                            segments.push(Segment {
                                text: std::mem::take(&mut current),
                                fg: existing.0,
                                bold: existing.1,
                            });
                            style = Some(next);
                        }
                        None => style = Some(next),
                        _ => {}
                    }
                    current.push(ch);
                }
                if let Some(done) = style {
                    segments.push(Segment {
                        text: current.trim_end_matches(' ').to_string(),
                        fg: done.0,
                        bold: done.1,
                    });
                }
                segments.retain(|s| !s.text.is_empty());
                if segments.is_empty() {
                    segments.push(Segment::plain("", pal.fg));
                }
                segments
            })
            .collect()
    }
    pub fn seed_banner(&mut self) {
        self.feed("UART Assistant serial terminal\r\n");
        self.feed("\x1b[90m[disconnected]\x1b[0m\r\n");
        self.feed("\x1b[1;32m>\x1b[0m ");
    }
}

/// 把颜色槽位解析为当前主题下的 RGB。
fn resolve(color: u16, inverse: bool, pal: &TermPalette) -> Rgb {
    if inverse {
        return pal.bg;
    }
    if color == DEFAULT_COLOR {
        return pal.fg;
    }
    if (color as usize) < 16 {
        return pal.ansi[color as usize];
    }
    xterm_color((color - 17) as u8)
}

fn xterm_color(index: u8) -> Rgb {
    if index < 16 {
        return crate::theme::TERM_DARK.ansi[(index % 8) as usize];
    }
    if index < 232 {
        let v = index - 16;
        let r = v / 36;
        let g = (v % 36) / 6;
        let b = v % 6;
        let level = |step: u8| -> u8 {
            if step == 0 {
                0
            } else {
                55 + step * 40
            }
        };
        return (level(r), level(g), level(b));
    }
    let gray = 8 + (index - 232) * 10;
    (gray, gray, gray)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banner_and_cursor_render_in_both_phases() {
        let mut terminal = TerminalEmulator::new(80, 24);
        terminal.seed_banner();
        for phase in [true, false] {
            let lines = terminal.render_lines(phase, &crate::theme::TERM_DARK);
            assert!(
                lines
                    .iter()
                    .any(|segments| segments.iter().any(|s| s.text.contains('█'))),
                "cursor block missing in phase {phase}"
            );
        }
    }
}
