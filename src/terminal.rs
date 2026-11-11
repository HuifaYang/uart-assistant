use crate::types::{color, Segment};
use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub fg: color::Rgb,
    pub bold: bool,
    pub inverse: bool,
}

impl Default for Attr {
    fn default() -> Self {
        Self {
            fg: (0xd6, 0xdc, 0xe5),
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

const ANSI: [color::Rgb; 8] = [
    (0x33, 0x3a, 0x45),
    (0xe0, 0x6c, 0x75),
    (0x98, 0xc3, 0x79),
    (0xe5, 0xc0, 0x7b),
    (0x61, 0xaf, 0xef),
    (0xc6, 0x78, 0xdd),
    (0x56, 0xb6, 0xc2),
    (0xdc, 0xe3, 0xeb),
];

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
        let mut attr = self.attr;
        if attr.inverse {
            attr.fg = (0x11, 0x15, 0x1b);
            attr.inverse = false;
        }
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
                30..=37 => self.attr.fg = ANSI[(code - 30) as usize],
                90..=97 => self.attr.fg = bright(ANSI[(code - 90) as usize]),
                39 => self.attr.fg = Attr::default().fg,
                38 if i + 2 < self.params.len() && self.params[i + 1] == 5 => {
                    self.attr.fg = xterm_color(self.params[i + 2] as u8);
                    i += 2;
                }
                40..=47 | 100..=107 | 48 | 49 => {}
                _ => {}
            }
            i += 1;
        }
    }

    pub fn render_lines(&self) -> Vec<Vec<Segment>> {
        self.screen
            .iter()
            .map(|row| {
                let mut segments: Vec<Segment> = Vec::new();
                let mut current = String::new();
                let mut current_attr: Option<Attr> = None;
                for cell in row {
                    if let Some(attr) = current_attr {
                        if attr != cell.attr {
                            segments.push(Segment {
                                text: std::mem::take(&mut current),
                                fg: attr.fg,
                                bold: attr.bold,
                            });
                            current_attr = Some(cell.attr);
                        }
                    } else {
                        current_attr = Some(cell.attr);
                    }
                    current.push(cell.ch);
                }
                if let Some(attr) = current_attr {
                    segments.push(Segment {
                        text: current.trim_end_matches(' ').to_string(),
                        fg: attr.fg,
                        bold: attr.bold,
                    });
                }
                segments.retain(|s| !s.text.is_empty());
                if segments.is_empty() {
                    segments.push(Segment::plain("", Attr::default().fg));
                }
                segments
            })
            .collect()
    }

    pub fn scrollback_lines(&self) -> Vec<Vec<Segment>> {
        self.scrollback
            .iter()
            .map(|row| {
                let text: String = row.iter().map(|c| c.ch).collect();
                vec![Segment::plain(text.trim_end().to_string(), Attr::default().fg)]
            })
            .collect()
    }
}

fn bright(base: color::Rgb) -> color::Rgb {
    (
        base.0.saturating_add(0x28),
        base.1.saturating_add(0x28),
        base.2.saturating_add(0x28),
    )
}

fn xterm_color(index: u8) -> color::Rgb {
    if index < 16 {
        return ANSI[(index % 8) as usize];
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
