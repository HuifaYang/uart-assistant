use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub mod color {
    pub type Rgb = (u8, u8, u8);
    pub const TEXT: Rgb = (0x1f, 0x23, 0x28);
    pub const MUTED: Rgb = (0x6b, 0x72, 0x80);
    pub const BLUE: Rgb = (0x25, 0x63, 0xeb);
    pub const TEAL: Rgb = (0x0f, 0x76, 0x6e);
    pub const AMBER: Rgb = (0xb4, 0x53, 0x09);
    pub const RED: Rgb = (0xc0, 0x39, 0x2b);
    pub const GREEN: Rgb = (0x00, 0x87, 0x5a);
    pub const WHITE: Rgb = (0xf8, 0xf9, 0xfb);
    pub const GRAY: Rgb = (0x9c, 0xa3, 0xaf);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Parity {
    #[default]
    None,
    Odd,
    Even,
}

impl Parity {
    pub fn index(self) -> i32 {
        match self {
            Parity::None => 0,
            Parity::Odd => 1,
            Parity::Even => 2,
        }
    }
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => Parity::Odd,
            2 => Parity::Even,
            _ => Parity::None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Parity::None => "None",
            Parity::Odd => "Odd",
            Parity::Even => "Even",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FlowControl {
    #[default]
    None,
    RtsCts,
    XonXoff,
}

impl FlowControl {
    pub fn index(self) -> i32 {
        match self {
            FlowControl::None => 0,
            FlowControl::RtsCts => 1,
            FlowControl::XonXoff => 2,
        }
    }
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => FlowControl::RtsCts,
            2 => FlowControl::XonXoff,
            _ => FlowControl::None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PortSettings {
    pub port: String,
    pub baud: u32,
    pub data_bits: u8,
    pub stop_bits: u8,
    pub parity: Parity,
    pub flow: FlowControl,
}

impl Default for PortSettings {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud: 115200,
            data_bits: 8,
            stop_bits: 1,
            parity: Parity::None,
            flow: FlowControl::None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeviceInfo {
    pub path: String,
    pub label: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Opening,
    Connected,
    Reconnecting,
    Closing,
}

impl ConnectionState {
    pub fn label(self) -> &'static str {
        match self {
            ConnectionState::Disconnected => "未连接",
            ConnectionState::Opening => "正在连接",
            ConnectionState::Connected => "已连接",
            ConnectionState::Reconnecting => "正在重连",
            ConnectionState::Closing => "正在关闭",
        }
    }
    pub fn connected(self) -> bool {
        self == ConnectionState::Connected
    }
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub text: String,
    pub fg: color::Rgb,
    pub bold: bool,
}

impl Segment {
    pub fn plain(text: impl Into<String>, fg: color::Rgb) -> Self {
        Self {
            text: text.into(),
            fg,
            bold: false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub segments: Vec<Segment>,
    pub text: String,
    pub timestamp: String,
}

impl LogLine {
    pub fn new(text: String, timestamp: String, fg: color::Rgb, keyword: &str) -> Self {
        Self {
            segments: split_highlight(&text, keyword, fg),
            text,
            timestamp,
        }
    }
}

pub fn split_highlight(text: &str, keyword: &str, fg: color::Rgb) -> Vec<Segment> {
    if text.is_empty() {
        return vec![Segment::plain("", fg)];
    }
    if keyword.is_empty() {
        return vec![Segment::plain(text.to_string(), fg)];
    }
    let needle = keyword.to_lowercase();
    let haystack = text.to_lowercase();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while cursor < text.len() {
        let Some(rel) = haystack[cursor..].find(&needle) else {
            break;
        };
        let start = cursor + rel;
        if !text.is_char_boundary(start) || !text.is_char_boundary(start + needle.len()) {
            break;
        }
        if start > cursor {
            out.push(Segment::plain(text[cursor..start].to_string(), fg));
        }
        out.push(Segment {
            text: text[start..start + needle.len()].to_string(),
            fg: color::AMBER,
            bold: true,
        });
        cursor = start + needle.len();
    }
    if cursor < text.len() {
        out.push(Segment::plain(text[cursor..].to_string(), fg));
    }
    if out.is_empty() {
        out.push(Segment::plain(text.to_string(), fg));
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    Text,
    Hex,
    AsciiHex,
}

impl DisplayMode {
    pub fn from_index(i: i32) -> Self {
        match i {
            1 => DisplayMode::Hex,
            2 => DisplayMode::AsciiHex,
            _ => DisplayMode::Text,
        }
    }
    pub fn index(self) -> i32 {
        match self {
            DisplayMode::Text => 0,
            DisplayMode::Hex => 1,
            DisplayMode::AsciiHex => 2,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Stm32Action {
    Reset,
    Bootloader,
}

#[derive(Clone, Debug)]
pub enum Command {
    Open(PortSettings),
    Close,
    Send(Vec<u8>),
    ConfigureLoop {
        data: Vec<u8>,
        interval_ms: u64,
        enabled: bool,
    },
    SetDtr(bool),
    SetRts(bool),
    Stm32(Stm32Action),
    StartXmodem {
        path: PathBuf,
        use_1k: bool,
    },
    CancelTransfer,
    StartRecording(PathBuf),
    StopRecording,
    StopReconnect,
    ProbeNow,
    SetAutoReconnect(bool),
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum SessionEvent {
    State(ConnectionState),
    Data(Vec<u8>),
    Stats { rx: u64, tx: u64 },
    Message(String),
    Transfer {
        active: bool,
        done: u64,
        total: u64,
        message: String,
    },
    Recording { active: bool, text: String },
}
