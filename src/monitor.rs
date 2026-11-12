use crate::types::{split_highlight, DisplayMode, LogLine, LogRole, LogSegment};
use encoding_rs::{Decoder, Encoding};
use std::collections::VecDeque;

pub const ENCODINGS: &[(&str, &str)] = &[
    ("UTF-8", "utf-8"),
    ("GB18030", "gb18030"),
    ("GBK", "gbk"),
    ("Big5", "big5"),
    ("Shift_JIS", "shift_jis"),
    ("Latin-1", "windows-1252"),
];

pub struct Monitor {
    decoder: Decoder,
    pub encoding_index: usize,
    pub mode: DisplayMode,
    pub timestamps: bool,
    pub autoscroll: bool,
    pub echo_tx: bool,
    pub paused: bool,
    pub highlight: String,
    pub line_limit: usize,
    lines: VecDeque<LogLine>,
    pending: String,
    line_stamp: String,
    cr_pending: bool,
    hex_buf: Vec<u8>,
    hex_offset: usize,
    dropped: usize,
}

impl Monitor {
    pub fn new() -> Self {
        Self {
            decoder: Encoding::for_label(ENCODINGS[0].1.as_bytes())
                .unwrap()
                .new_decoder(),
            encoding_index: 0,
            mode: DisplayMode::Text,
            timestamps: false,
            autoscroll: true,
            echo_tx: false,
            paused: false,
            highlight: String::new(),
            line_limit: 10000,
            lines: VecDeque::new(),
            pending: String::new(),
            line_stamp: String::new(),
            cr_pending: false,
            hex_buf: Vec::new(),
            hex_offset: 0,
            dropped: 0,
        }
    }

    pub fn set_encoding(&mut self, index: usize) {
        let clamped = index.min(ENCODINGS.len() - 1);
        self.encoding_index = clamped;
        if let Some(enc) = Encoding::for_label(ENCODINGS[clamped].1.as_bytes()) {
            self.decoder = enc.new_decoder();
        }
    }

    pub fn set_mode(&mut self, mode: DisplayMode) {
        if self.mode == mode {
            return;
        }
        self.flush_pending("");
        self.mode = mode;
        self.hex_buf.clear();
        self.hex_offset = 0;
        if let Some(enc) = Encoding::for_label(ENCODINGS[self.encoding_index].1.as_bytes()) {
            self.decoder = enc.new_decoder();
        }
    }

    pub fn set_line_limit(&mut self, limit: usize) {
        self.line_limit = limit.clamp(200, 30000);
        self.trim();
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.pending.clear();
        self.cr_pending = false;
        self.hex_buf.clear();
        self.hex_offset = 0;
        self.dropped = 0;
    }

    fn trim(&mut self) {
        while self.lines.len() > self.line_limit {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    fn push(&mut self, segments: Vec<LogSegment>, text: String, stamp: String) {
        if self.paused {
            return;
        }
        self.lines.push_back(LogLine {
            segments,
            text,
            timestamp: stamp,
        });
        self.trim();
    }

    fn stamp_prefix(&self, stamp: &str) -> String {
        if self.timestamps {
            format!("[{}] ", stamp)
        } else {
            String::new()
        }
    }

    fn flush_pending(&mut self, stamp: &str) {
        if self.pending.is_empty() && !self.cr_pending {
            return;
        }
        if self.mode == DisplayMode::Text {
            let stamp = if self.line_stamp.is_empty() {
                stamp.to_string()
            } else {
                self.line_stamp.clone()
            };
            let prefix = self.stamp_prefix(&stamp);
            let keyword = self.highlight.clone();
            let mut segments = Vec::new();
            if !prefix.is_empty() {
                segments.push(LogSegment {
                    text: prefix.clone(),
                    role: LogRole::Muted,
                    bold: false,
                });
            }
            segments.extend(split_highlight(&self.pending, &keyword, LogRole::Text));
            let text = format!("{}{}", prefix, self.pending);
            self.push(segments, text, stamp);
        } else {
            let chunk = std::mem::take(&mut self.hex_buf);
            self.push_hex_chunk(&chunk);
        }
        self.pending.clear();
        self.line_stamp.clear();
        self.cr_pending = false;
    }

    pub fn feed_rx(&mut self, data: &[u8], stamp: &str) {
        if data.is_empty() {
            return;
        }
        match self.mode {
            DisplayMode::Text => {
                let mut decoded = String::new();
                crate::types::decode_into(&mut self.decoder, data, &mut decoded);
                for ch in decoded.chars() {
                    if ch == '\r' {
                        self.cr_pending = true;
                        continue;
                    }
                    if self.cr_pending {
                        self.flush_pending(stamp);
                    }
                    if ch == '\n' {
                        self.flush_pending(stamp);
                        continue;
                    }
                    if ch < ' ' && ch != '\t' {
                        continue;
                    }
                    if self.pending.is_empty() {
                        self.line_stamp = stamp.to_string();
                    }
                    self.pending.push(ch);
                }
            }
            DisplayMode::Hex | DisplayMode::AsciiHex => {
                self.hex_buf.extend_from_slice(data);
                while self.hex_buf.len() >= 16 {
                    let chunk: Vec<u8> = self.hex_buf.drain(..16).collect();
                    self.push_hex_chunk(&chunk);
                }
            }
        }
    }

    fn push_hex_chunk(&mut self, chunk: &[u8]) {
        let offset = self.hex_offset;
        self.hex_offset += chunk.len();
        let mut hex = String::new();
        for (i, byte) in chunk.iter().enumerate() {
            if i == 8 {
                hex.push(' ');
            }
            hex.push_str(&format!("{:02X} ", byte));
        }
        let text = if self.mode == DisplayMode::AsciiHex {
            let ascii: String = chunk
                .iter()
                .map(|&b| {
                    if (0x20..0x7f).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                })
                .collect();
            format!("{:08X}  {:<49} |{}|", offset, hex, ascii)
        } else {
            format!("{:08X}  {}", offset, hex.trim_end())
        };
        let keyword = self.highlight.clone();
        let segments = split_highlight(&text, &keyword, LogRole::Hex);
        self.push(segments, text, String::new());
    }

    pub fn feed_tx(&mut self, text: &str, stamp: &str) {
        if self.paused {
            return;
        }
        let prefix = format!("{}TX> ", self.stamp_prefix(stamp));
        let keyword = self.highlight.clone();
        let mut segments = vec![LogSegment {
            text: prefix.clone(),
            role: LogRole::Tx,
            bold: true,
        }];
        segments.extend(split_highlight(text, &keyword, LogRole::Tx));
        self.push(segments, format!("{}{}", prefix, text), stamp.to_string());
    }

    pub fn visible_lines(&self, filter: &str) -> Vec<LogLine> {
        if filter.is_empty() {
            return self.lines.iter().cloned().collect();
        }
        let needle = filter.to_lowercase();
        self.lines
            .iter()
            .filter(|line| line.text.to_lowercase().contains(&needle))
            .map(|line| LogLine {
                segments: split_highlight(&line.text, filter, LogRole::Text),
                text: line.text.clone(),
                timestamp: line.timestamp.clone(),
            })
            .collect()
    }

    pub fn plain_text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    /// 按索引读取一行（0 为最早一行），供增量推送到前端。
    pub fn line_at(&self, index: usize) -> Option<&LogLine> {
        self.lines.get(index)
    }

    /// 清空后重新推送时，前端需要知道当前行数。
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_probe() {
        let mut dec = Encoding::for_label(b"utf-8").unwrap().new_decoder();
        let mut s = String::new();
        let r = dec.decode_to_string(b"abc\r\ndef\r\n", &mut s, false);
        println!("PROBE r={:?} decoded={:?} n={}", r, s, s.len());
        let mut m = Monitor::new();
        m.feed_rx(b"abc\r\ndef\r\n", "t");
        println!("PROBE lines={} pending={:?} cr={}", m.lines.len(), m.pending, m.cr_pending);
    }

    #[test]
    fn crlf_lines_are_flushed() {
        let mut m = Monitor::new();
        m.feed_rx(b"a\r\nb\r\nc\r\n", "12:00");
        assert_eq!(m.line_count(), 3, "lines: {:?}", m.visible_lines("").iter().map(|l| l.text.clone()).collect::<Vec<_>>());
    }
}