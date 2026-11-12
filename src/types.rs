use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 把一段字节解码进 `out`。
///
/// `encoding_rs` 的 `decode_to_string` 不会为输出扩容：目标 String 容量不足时
/// 直接返回 `OutputFull` 且不写入任何内容。这里预留足够容量，避免静默丢数据。
pub fn decode_into(decoder: &mut encoding_rs::Decoder, bytes: &[u8], out: &mut String) {
    out.reserve(bytes.len().saturating_mul(4).saturating_add(16));
    let _ = decoder.decode_to_string(bytes, out, false);
}

pub mod color {
    pub type Rgb = (u8, u8, u8);
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

/// 日志文本的语义角色：具体颜色在渲染时按当前主题解析。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogRole {
    Text,
    Muted,
    Tx,
    Hex,
    Highlight,
}

#[derive(Clone, Debug)]
pub struct LogSegment {
    pub text: String,
    pub role: LogRole,
    pub bold: bool,
}

#[derive(Clone, Debug)]
pub struct LogLine {
    pub segments: Vec<LogSegment>,
    pub text: String,
    pub timestamp: String,
}

pub fn split_highlight(text: &str, keyword: &str, role: LogRole) -> Vec<LogSegment> {
    let plain = |s: &str| LogSegment {
        text: s.to_string(),
        role,
        bold: false,
    };
    if text.is_empty() {
        return vec![plain("")];
    }
    if keyword.is_empty() {
        return vec![plain(text)];
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
            out.push(plain(&text[cursor..start]));
        }
        out.push(LogSegment {
            text: text[start..start + needle.len()].to_string(),
            role: LogRole::Highlight,
            bold: true,
        });
        cursor = start + needle.len();
    }
    if cursor < text.len() {
        out.push(plain(&text[cursor..]));
    }
    if out.is_empty() {
        out.push(plain(text));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    Monitor,
    Terminal,
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
