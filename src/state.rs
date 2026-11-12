use crate::config::{Config, MacroItem};
use crate::monitor::{Monitor, ENCODINGS};
use crate::port::scan_devices;
use crate::session::Session;
use crate::terminal::TerminalEmulator;
use crate::theme;
use crate::types::*;
use encoding_rs::{Decoder, Encoding};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub fn parse_hex_rgb(value: &str) -> Option<theme::Rgb> {
    let s = value.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

fn now_stamp() -> String {
    chrono::Local::now().format("%H:%M:%S%.3f").to_string()
}

fn decoder_for(index: usize) -> Decoder {
    Encoding::for_label(ENCODINGS[index.min(ENCODINGS.len() - 1)].1.as_bytes())
        .unwrap()
        .new_decoder()
}

fn parse_send(text: &str, hex: bool, append_newline: bool) -> Result<Vec<u8>, String> {
    if hex {
        let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        if cleaned.is_empty() {
            return Ok(Vec::new());
        }
        if cleaned.len() % 2 != 0 {
            return Err("HEX 字符串长度必须为偶数".into());
        }
        let mut bytes = Vec::with_capacity(cleaned.len() / 2);
        for i in (0..cleaned.len()).step_by(2) {
            bytes.push(
                u8::from_str_radix(&cleaned[i..i + 2], 16)
                    .map_err(|_| format!("非法 HEX 字节: {}", &cleaned[i..i + 2]))?,
            );
        }
        return Ok(bytes);
    }
    let mut bytes = text.replace("\\r", "\r").replace("\\n", "\n").into_bytes();
    if append_newline {
        bytes.extend_from_slice(b"\r\n");
    }
    Ok(bytes)
}

pub struct UiSession {
    pub kind: SessionKind,
    pub session: Session,
    pub monitor: Monitor,
    pub terminal: TerminalEmulator,
    term_decoder: Decoder,
    pub title: String,
    pub settings: PortSettings,
    pub loop_enabled: bool,
    pub loop_interval_ms: u64,
    pub xmodem_path: Option<PathBuf>,
    pub use_1k: bool,
    pub log_dirty: bool,
    pub term_dirty: bool,
    pub transfer_done: u64,
    pub transfer_total: u64,
    pub cursor_on: bool,
    last_blink: Instant,
    sent: usize,
    last_dropped: usize,
}

impl UiSession {
    fn new(id: usize, kind: SessionKind, settings: PortSettings, cfg: &Config) -> Self {
        let mut monitor = Monitor::new();
        monitor.timestamps = cfg.monitor.timestamps;
        monitor.autoscroll = cfg.monitor.autoscroll;
        monitor.echo_tx = cfg.monitor.echo_tx;
        monitor.highlight = cfg.monitor.highlight.clone();
        monitor.line_limit = cfg.monitor.line_limit.max(200) as usize;
        let encoding_index = (cfg.monitor.encoding_index.max(0) as usize).min(ENCODINGS.len() - 1);
        monitor.set_encoding(encoding_index);
        monitor.set_mode(DisplayMode::from_index(cfg.monitor.mode));
        let mut terminal = TerminalEmulator::new(100, 30);
        terminal.seed_banner();
        Self {
            kind,
            session: Session::new(id, settings.clone(), cfg.auto_reconnect),
            monitor,
            terminal,
            term_decoder: decoder_for(encoding_index),
            title: if kind == SessionKind::Terminal {
                format!("终端 {}", id + 1)
            } else {
                format!("会话 {}", id + 1)
            },
            settings,
            loop_enabled: false,
            loop_interval_ms: cfg.monitor.loop_interval_ms.max(50) as u64,
            xmodem_path: None,
            use_1k: false,
            log_dirty: true,
            term_dirty: true,
            transfer_done: 0,
            transfer_total: 0,
            cursor_on: true,
            last_blink: Instant::now(),
            sent: 0,
            last_dropped: 0,
        }
    }
}

pub struct App {
    pub cfg: Config,
    pub sessions: Vec<UiSession>,
    pub active: usize,
    next_id: usize,
    pub devices: Vec<DeviceInfo>,
    signature: Vec<String>,
    pub out: Vec<Value>,
    pub font: f32,
    scan_tx: mpsc::Sender<Vec<DeviceInfo>>,
    scan_rx: mpsc::Receiver<Vec<DeviceInfo>>,
    scanning: bool,
    next_scan: Instant,
    last_status: Instant,
    status_sig: String,
    cfg_dirty: bool,
    last_save: Instant,
    pub term_colors: &'static theme::TermPalette,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        let (scan_tx, scan_rx) = mpsc::channel();
        let devices = scan_devices();
        let mut settings = cfg.last_settings.clone();
        if !devices.iter().any(|d| d.path == settings.port) {
            if let Some(first) = devices.first() {
                settings.port = first.path.clone();
            }
        }
        let term_colors = theme::term_palette(cfg.dark);
        let session = UiSession::new(0, SessionKind::Monitor, settings, &cfg);
        let font = cfg.monitor.log_font_size;
        Self {
            cfg,
            sessions: vec![session],
            active: 0,
            next_id: 1,
            devices,
            signature: Vec::new(),
            out: Vec::new(),
            font,
            scan_tx,
            scan_rx,
            scanning: false,
            next_scan: Instant::now() + Duration::from_secs(2),
            last_status: Instant::now() - Duration::from_secs(1),
            status_sig: String::new(),
            cfg_dirty: false,
            last_save: Instant::now(),
            term_colors,
        }
    }

    pub fn boot(&mut self) {
        self.out.push(json!({"t": "theme", "theme": theme::theme_json(&self.cfg)}));
        self.push_devices();
        self.push_tabs();
        self.push_session();
        self.push_status_force();
        self.push_macros();
    }

    // ── 推送 ────────────────────────────────────────────
    fn push_devices(&mut self) {
        let list: Vec<Value> = self
            .devices
            .iter()
            .map(|d| json!({"path": d.path, "label": d.label, "detail": d.detail}))
            .collect();
        let index = self
            .devices
            .iter()
            .position(|d| d.path == self.sessions[self.active].settings.port)
            .map(|i| i as i32)
            .unwrap_or(-1);
        self.out.push(json!({"t": "devices", "list": list, "index": index}));
    }

    fn push_tabs(&mut self) {
        let tabs: Vec<Value> = self
            .sessions
            .iter()
            .map(|s| {
                json!({
                    "title": s.title,
                    "connected": s.session.state.connected(),
                    "kind": if s.kind == SessionKind::Terminal { 1 } else { 0 },
                })
            })
            .collect();
        self.out.push(json!({"t": "tabs", "tabs": tabs, "active": self.active as i32}));
    }

    fn push_session(&mut self) {
        let s = &self.sessions[self.active];
        let value = json!({
            "t": "session",
            "kind": if s.kind == SessionKind::Terminal { 1 } else { 0 },
            "title": s.title,
            "port": s.settings.port,
            "baud": s.settings.baud,
            "dataBits": s.settings.data_bits,
            "stopBits": s.settings.stop_bits,
            "parity": s.settings.parity.index(),
            "flow": s.settings.flow.index(),
            "mode": s.monitor.mode.index(),
            "encoding": s.monitor.encoding_index as i32,
            "encodings": ENCODINGS.iter().map(|e| e.0).collect::<Vec<_>>(),
            "timestamps": s.monitor.timestamps,
            "autoscroll": s.monitor.autoscroll,
            "paused": s.monitor.paused,
            "echoTx": s.monitor.echo_tx,
            "highlight": s.monitor.highlight,
            "lineLimit": s.monitor.line_limit as i32,
            "fontSize": self.font,
            "appendNewline": self.cfg.monitor.append_newline,
            "hexSend": self.cfg.monitor.hex_send,
            "autoReconnect": self.cfg.auto_reconnect,
            "loop": s.loop_enabled,
            "loopInterval": s.loop_interval_ms as i32,
            "use1k": s.use_1k,
            "xmodemFile": s.xmodem_path.as_ref().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_else(|| "未选择文件".into()),
            "macros": self.cfg.macros.iter().map(|m| json!({"name": m.name, "hex": m.hex})).collect::<Vec<_>>(),
        });
        self.out.push(value);
        self.term_colors = theme::term_palette(self.cfg.dark);
        self.push_log_reset();
        self.push_term();
    }

    fn push_macros(&mut self) {
        let text = self
            .cfg
            .macros
            .iter()
            .map(|m| {
                format!(
                    "{} | {} | {}",
                    m.name,
                    m.command.replace('\r', "\\r").replace('\n', "\\n"),
                    if m.hex { "1" } else { "0" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        self.out.push(json!({"t": "macros", "text": text}));
    }

    pub fn push_log_reset(&mut self) {
        let s = &self.sessions[self.active];
        let total = s.monitor.line_count();
        let mut append = Vec::with_capacity(total);
        for i in 0..total {
            if let Some(line) = s.monitor.line_at(i) {
                append.push(line_json(line));
            }
        }
        self.out.push(json!({
            "t": "log", "reset": true, "drop": 0, "append": append, "total": total
        }));
        let s = &mut self.sessions[self.active];
        s.sent = total;
        s.last_dropped = s.monitor.dropped();
    }

    fn push_log(&mut self) {
        let s = &mut self.sessions[self.active];
        let total = s.monitor.line_count();
        let dropped = s.monitor.dropped();
        let reset = total < s.sent;
        let drop = if reset {
            0
        } else {
            dropped.saturating_sub(s.last_dropped)
        };
        let from = if reset { 0 } else { s.sent.saturating_sub(drop) };
        if total <= from && drop == 0 && !reset {
            return;
        }
        let mut append = Vec::with_capacity(total.saturating_sub(from));
        for i in from..total {
            if let Some(line) = s.monitor.line_at(i) {
                append.push(line_json(line));
            }
        }
        let sent = total;
        s.sent = sent;
        s.last_dropped = dropped;
        self.out.push(json!({
            "t": "log", "reset": reset, "drop": drop, "append": append, "total": total
        }));
    }

    fn push_term(&mut self) {
        let show_cursor = self.sessions[self.active].cursor_on;
        let pal = self.term_colors;
        let rows: Vec<Value> = self.sessions[self.active]
            .terminal
            .render_lines(show_cursor, pal)
            .iter()
            .map(|line| {
                let segs: Vec<Value> = line
                    .iter()
                    .map(|seg| {
                        json!([
                            seg.text,
                            theme::hex(seg.fg),
                            if seg.bold { 1 } else { 0 }
                        ])
                    })
                    .collect();
                Value::Array(segs)
            })
            .collect();
        self.out.push(json!({"t": "term", "rows": rows}));
    }

    fn push_status_force(&mut self) {
        let s = &self.sessions[self.active];
        let sig = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            s.session.state.label(),
            s.session.rx_bytes,
            s.session.tx_bytes,
            s.monitor.line_count(),
            s.session.last_message,
            s.session.recording_active,
            s.session.transferring,
            s.session.transfer_text
        );
        self.status_sig = sig;
        let progress = if s.transfer_total > 0 {
            (s.transfer_done as f64 / s.transfer_total as f64).min(1.0)
        } else {
            0.0
        };
        self.out.push(json!({
            "t": "status",
            "state": s.session.state.label(),
            "connected": s.session.state.connected(),
            "rx": s.session.rx_bytes,
            "tx": s.session.tx_bytes,
            "lines": s.monitor.line_count(),
            "msg": s.session.last_message,
            "port": s.settings.port,
            "baud": s.settings.baud,
            "recording": s.session.recording_active,
            "recordText": s.session.recording_text,
            "transfer": s.session.transferring,
            "transferText": s.session.transfer_text,
            "progress": progress,
        }));
        // 标签上的连接点
        self.push_tabs();
    }

    fn push_status(&mut self) {
        let s = &self.sessions[self.active];
        let sig = format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            s.session.state.label(),
            s.session.rx_bytes,
            s.session.tx_bytes,
            s.monitor.line_count(),
            s.session.last_message,
            s.session.recording_active,
            s.session.transferring,
            s.session.transfer_text
        );
        if sig == self.status_sig {
            return;
        }
        self.push_status_force();
    }

    // ── 命令 ────────────────────────────────────────────
    pub fn command(&mut self, body: &str) {
        let Ok(v) = serde_json::from_str::<Value>(body) else {
            return;
        };
        let cmd = v.get("cmd").and_then(|c| c.as_str()).unwrap_or("");
        match cmd {
            "init" => self.boot(),
            "rescan" => {
                self.next_scan = Instant::now() - Duration::from_secs(1);
                self.signature.clear();
            }
            "device" => {
                if let Some(i) = v.get("index").and_then(|x| x.as_i64()) {
                    if let Some(dev) = self.devices.get(i.max(0) as usize) {
                        let path = dev.path.clone();
                        let saved = self.cfg.profile_for(&path).unwrap_or_default();
                        let s = &mut self.sessions[self.active];
                        s.settings = if saved.port.is_empty() {
                            PortSettings { port: path, ..self.cfg.last_settings.clone() }
                        } else {
                            saved
                        };
                        self.push_session();
                        self.push_devices();
                    }
                }
            }
            "baud" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_u64()) {
                    self.sessions[self.active].settings.baud = b as u32;
                    self.remember();
                    self.push_session();
                }
            }
            "params" => {
                let s = &mut self.sessions[self.active];
                if let Some(x) = v.get("dataBits").and_then(|x| x.as_u64()) {
                    s.settings.data_bits = x.clamp(5, 8) as u8;
                }
                if let Some(x) = v.get("stopBits").and_then(|x| x.as_u64()) {
                    s.settings.stop_bits = if x >= 2 { 2 } else { 1 };
                }
                if let Some(x) = v.get("parity").and_then(|x| x.as_i64()) {
                    s.settings.parity = Parity::from_index(x as i32);
                }
                if let Some(x) = v.get("flow").and_then(|x| x.as_i64()) {
                    s.settings.flow = FlowControl::from_index(x as i32);
                }
                self.remember();
                self.push_session();
            }
            "connect" => self.toggle_connect(),
            "new-session" => {
                let kind = if v.get("kind").and_then(|x| x.as_i64()) == Some(1) {
                    SessionKind::Terminal
                } else {
                    SessionKind::Monitor
                };
                self.add_session(kind);
            }
            "close-session" => {
                if let Some(i) = v.get("index").and_then(|x| x.as_i64()) {
                    self.close_session(i as usize);
                }
            }
            "select-session" => {
                if let Some(i) = v.get("index").and_then(|x| x.as_i64()) {
                    let i = i as usize;
                    if i < self.sessions.len() {
                        self.active = i;
                        self.push_tabs();
                        self.push_devices();
                        self.push_session();
                        self.push_status_force();
                    }
                }
            }
            "rename-session" => {
                if let Some(i) = v.get("index").and_then(|x| x.as_i64()) {
                    if let Some(title) = v.get("title").and_then(|x| x.as_str()) {
                        if let Some(s) = self.sessions.get_mut(i.max(0) as usize) {
                            if !title.trim().is_empty() {
                                s.title = title.trim().to_string();
                            }
                        }
                        self.push_tabs();
                    }
                }
            }
            "send" => {
                let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                self.send_text(text, None, true);
            }
            "send-macro" => {
                if let Some(i) = v.get("index").and_then(|x| x.as_i64()) {
                    self.send_macro(i as usize);
                }
            }
            "clear" => {
                let s = &mut self.sessions[self.active];
                s.monitor.clear();
                s.sent = 0;
                s.last_dropped = 0;
                self.out.push(json!({"t": "log", "reset": true, "drop": 0, "append": [], "total": 0}));
            }
            "mode" => {
                if let Some(i) = v.get("value").and_then(|x| x.as_i64()) {
                    let mode = DisplayMode::from_index(i as i32);
                    for s in &mut self.sessions {
                        s.monitor.set_mode(mode);
                        s.log_dirty = true;
                    }
                    self.cfg.monitor.mode = i as i32;
                    self.cfg_dirty = true;
                    self.push_session();
                    self.push_log_reset();
                }
            }
            "encoding" => {
                if let Some(i) = v.get("value").and_then(|x| x.as_i64()) {
                    let i = (i.max(0) as usize).min(ENCODINGS.len() - 1);
                    for s in &mut self.sessions {
                        s.monitor.set_encoding(i);
                        s.term_decoder = decoder_for(i);
                        s.term_dirty = true;
                    }
                    self.cfg.monitor.encoding_index = i as i32;
                    self.cfg_dirty = true;
                    self.push_session();
                }
            }
            "timestamps" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    for s in &mut self.sessions {
                        s.monitor.timestamps = b;
                    }
                    self.cfg.monitor.timestamps = b;
                    self.cfg_dirty = true;
                }
            }
            "autoscroll" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    for s in &mut self.sessions {
                        s.monitor.autoscroll = b;
                    }
                    self.cfg.monitor.autoscroll = b;
                    self.cfg_dirty = true;
                }
            }
            "paused" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    for s in &mut self.sessions {
                        s.monitor.paused = b;
                    }
                    self.push_session();
                }
            }
            "echo" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    for s in &mut self.sessions {
                        s.monitor.echo_tx = b;
                    }
                    self.cfg.monitor.echo_tx = b;
                    self.cfg_dirty = true;
                }
            }
            "highlight" => {
                if let Some(text) = v.get("value").and_then(|x| x.as_str()) {
                    for s in &mut self.sessions {
                        s.monitor.highlight = text.to_string();
                    }
                    self.cfg.monitor.highlight = text.to_string();
                    self.cfg_dirty = true;
                }
            }
            "hex-send" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    self.cfg.monitor.hex_send = b;
                    self.cfg_dirty = true;
                    self.push_session();
                }
            }
            "append-newline" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    self.cfg.monitor.append_newline = b;
                    self.cfg_dirty = true;
                    self.push_session();
                }
            }
            "line-limit" => {
                if let Some(n) = v.get("value").and_then(|x| x.as_i64()) {
                    let n = (n.max(200) as usize).min(30000);
                    for s in &mut self.sessions {
                        s.monitor.set_line_limit(n);
                    }
                    self.cfg.monitor.line_limit = n as i32;
                    self.cfg_dirty = true;
                    self.push_session();
                }
            }
            "font" => {
                if let Some(n) = v.get("value").and_then(|x| x.as_f64()) {
                    self.font = (n as f32).clamp(9.0, 32.0);
                    self.cfg.monitor.log_font_size = self.font;
                    self.cfg_dirty = true;
                }
            }
            "loop" => {
                let enabled = v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
                let interval = v.get("interval").and_then(|x| x.as_i64()).unwrap_or(1000).max(50) as u64;
                let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("");
                let hex = self.cfg.monitor.hex_send;
                let append = self.cfg.monitor.append_newline;
                let data = parse_send(text, hex, append).unwrap_or_default();
                let s = &mut self.sessions[self.active];
                s.loop_enabled = enabled;
                s.loop_interval_ms = interval;
                s.session.send(Command::ConfigureLoop { data, interval_ms: interval, enabled });
                self.cfg.monitor.loop_interval_ms = interval as i32;
                self.cfg_dirty = true;
            }
            "save-log" => self.save_log(),
            "toggle-record" => self.toggle_record(),
            "dtr" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    self.sessions[self.active].session.send(Command::SetDtr(b));
                }
            }
            "rts" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    self.sessions[self.active].session.send(Command::SetRts(b));
                }
            }
            "stm32" => {
                let boot = v.get("mode").and_then(|x| x.as_i64()) == Some(1);
                self.sessions[self.active].session.send(Command::Stm32(if boot {
                    Stm32Action::Bootloader
                } else {
                    Stm32Action::Reset
                }));
            }
            "use-1k" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    self.sessions[self.active].use_1k = b;
                }
            }
            "xmodem-choose" => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.sessions[self.active].xmodem_path = Some(path);
                    self.push_session();
                }
            }
            "xmodem-start" => {
                let s = &self.sessions[self.active];
                let Some(path) = s.xmodem_path.clone() else {
                    self.sessions[self.active].session.last_message = "请先选择 XModem 文件".into();
                    return;
                };
                let use_1k = s.use_1k;
                self.sessions[self.active].session.send(Command::StartXmodem { path, use_1k });
            }
            "xmodem-cancel" => {
                self.sessions[self.active].session.send(Command::CancelTransfer);
            }
            "terminal-key" => {
                let text = v.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let enter = v.get("enter").and_then(|x| x.as_bool()).unwrap_or(false) || text == "\r";
                self.send_terminal_key(text, enter);
            }
            "auto-reconnect" => {
                if let Some(b) = v.get("value").and_then(|x| x.as_bool()) {
                    self.cfg.auto_reconnect = b;
                    self.cfg_dirty = true;
                    for s in &self.sessions {
                        s.session.send(Command::SetAutoReconnect(b));
                    }
                }
            }
            "settings-save" => {
                if let Some(dark) = v.get("dark").and_then(|x| x.as_bool()) {
                    self.cfg.dark = dark;
                }
                if let Some(accent) = v.get("accent").and_then(|x| x.as_i64()) {
                    self.cfg.accent = accent as i32;
                }
                if let Some(text) = v.get("textColor").and_then(|x| x.as_str()) {
                    self.cfg.text_color = text.to_string();
                }
                self.cfg.save();
                self.term_colors = theme::term_palette(self.cfg.dark);
                self.out.push(json!({"t": "theme", "theme": theme::theme_json(&self.cfg)}));
                self.sessions[self.active].term_dirty = true;
                self.push_term();
            }
            "settings-preview" => {
                if let Some(dark) = v.get("dark").and_then(|x| x.as_bool()) {
                    let mut preview = self.cfg.clone();
                    preview.dark = dark;
                    if let Some(accent) = v.get("accent").and_then(|x| x.as_i64()) {
                        preview.accent = accent as i32;
                    }
                    if let Some(text) = v.get("textColor").and_then(|x| x.as_str()) {
                        preview.text_color = text.to_string();
                    }
                    self.out.push(json!({"t": "theme", "theme": theme::theme_json(&preview)}));
                }
            }
            "macros-save" => {
                if let Some(text) = v.get("text").and_then(|x| x.as_str()) {
                    let mut macros = Vec::new();
                    for line in text.lines() {
                        let parts: Vec<&str> = line.splitn(3, '|').collect();
                        if parts.len() < 2 {
                            continue;
                        }
                        let name = parts[0].trim().to_string();
                        if name.is_empty() {
                            continue;
                        }
                        macros.push(MacroItem {
                            name,
                            command: parts[1].trim().to_string(),
                            hex: parts.get(2).map(|x| x.trim() == "1").unwrap_or(false),
                        });
                    }
                    self.cfg.macros = macros;
                    self.cfg_dirty = true;
                    self.push_macros();
                    self.push_session();
                }
            }
            "macros-get" => self.push_macros(),
            _ => {}
        }
    }

    // ── 会话动作 ────────────────────────────────────────
    fn add_session(&mut self, kind: SessionKind) {
        // 新会话沿用当前会话的端口与参数，避免用配置里可能已拔掉的旧端口。
        let settings = self.sessions[self.active].settings.clone();
        let id = self.next_id;
        self.next_id += 1;
        self.sessions.push(UiSession::new(id, kind, settings, &self.cfg));
        self.active = self.sessions.len() - 1;
        self.push_tabs();
        self.push_devices();
        self.push_session();
        self.push_status_force();
    }

    fn close_session(&mut self, index: usize) {
        if self.sessions.len() <= 1 || index >= self.sessions.len() {
            return;
        }
        let mut removed = self.sessions.remove(index);
        removed.session.shutdown();
        if self.active >= self.sessions.len() {
            self.active = self.sessions.len() - 1;
        } else if self.active > index {
            self.active -= 1;
        }
        self.push_tabs();
        self.push_devices();
        self.push_session();
        self.push_status_force();
    }

    fn toggle_connect(&mut self) {
        let s = &mut self.sessions[self.active];
        if s.session.state.connected() || s.session.state == ConnectionState::Opening {
            s.session.send(Command::Close);
            return;
        }
        if s.settings.port.is_empty() {
            s.session.last_message = "请选择串口设备".into();
            return;
        }
        let settings = s.settings.clone();
        s.session.send(Command::SetAutoReconnect(self.cfg.auto_reconnect));
        s.session.send(Command::Open(settings.clone()));
        self.cfg.remember(&settings.port, &settings);
        self.cfg_dirty = true;
    }

    fn remember(&mut self) {
        let settings = self.sessions[self.active].settings.clone();
        self.sessions[self.active].session.settings = settings.clone();
        self.cfg.remember(&settings.port, &settings);
        self.cfg_dirty = true;
    }

    fn send_text(&mut self, text: &str, hex_override: Option<bool>, guard_newline: bool) {
        if text.is_empty() {
            return;
        }
        let hex = hex_override.unwrap_or(self.cfg.monitor.hex_send);
        let append = guard_newline && self.cfg.monitor.append_newline && !hex;
        let bytes = match parse_send(text, hex, append) {
            Ok(bytes) if !bytes.is_empty() => bytes,
            Ok(_) => return,
            Err(err) => {
                self.sessions[self.active].session.last_message = err;
                return;
            }
        };
        let s = &mut self.sessions[self.active];
        s.session.send(Command::Send(bytes));
        if s.monitor.echo_tx {
            s.monitor.feed_tx(text, &now_stamp());
        }
    }

    fn send_macro(&mut self, index: usize) {
        let Some(item) = self.cfg.macros.get(index).cloned() else {
            return;
        };
        let bytes = match parse_send(&item.command, item.hex, false) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.sessions[self.active].session.last_message = err;
                return;
            }
        };
        let s = &mut self.sessions[self.active];
        s.session.send(Command::Send(bytes));
        if s.monitor.echo_tx {
            s.monitor.feed_tx(&item.command, &now_stamp());
        }
    }

    fn send_terminal_key(&mut self, text: String, enter: bool) {
        let bytes: Vec<u8> = if enter {
            vec![b'\r']
        } else if text.is_empty() {
            return;
        } else {
            text.as_bytes().to_vec()
        };
        let s = &mut self.sessions[self.active];
        s.session.send(Command::Send(bytes));
        if enter {
            s.terminal.feed("\r\n");
        } else {
            s.terminal.feed(&text);
        }
        s.term_dirty = true;
    }

    fn save_log(&mut self) {
        let text = self.sessions[self.active].monitor.plain_text();
        if let Some(path) = rfd::FileDialog::new().set_file_name("uart-log.txt").save_file() {
            let _ = std::fs::write(path, text);
        }
    }

    fn toggle_record(&mut self) {
        let s = &mut self.sessions[self.active];
        if s.session.recording_active {
            s.session.send(Command::StopRecording);
        } else if let Some(dir) = rfd::FileDialog::new().pick_folder() {
            s.session.send(Command::StartRecording(dir));
        }
    }

    // ── 事件循环 ────────────────────────────────────────
    pub fn tick(&mut self) {
        let stamp = now_stamp();
        let now = Instant::now();
        for s in self.sessions.iter_mut() {
            for event in s.session.drain_events() {
                match event {
                    SessionEvent::Data(bytes) => {
                        s.monitor.feed_rx(&bytes, &stamp);
                        let mut decoded = String::new();
                        decode_into(&mut s.term_decoder, &bytes, &mut decoded);
                        s.terminal.feed(&decoded);
                        s.log_dirty = true;
                        s.term_dirty = true;
                    }
                    SessionEvent::State(state) => {
                        if state == ConnectionState::Connected
                            && s.session.state != ConnectionState::Connected
                        {
                            let (cols, rows) = (s.terminal.cols, s.terminal.rows);
                            s.terminal = TerminalEmulator::new(cols, rows);
                            s.term_dirty = true;
                        }
                        s.session.state = state;
                        s.log_dirty = true;
                    }
                    SessionEvent::Stats { rx, tx } => {
                        s.session.rx_bytes = rx;
                        s.session.tx_bytes = tx;
                    }
                    SessionEvent::Message(message) => {
                        s.session.last_message = message;
                    }
                    SessionEvent::Transfer { active, done, total, message } => {
                        s.session.transferring = active;
                        s.session.transfer_text = message;
                        s.transfer_done = done;
                        s.transfer_total = total;
                    }
                    SessionEvent::Recording { active, text } => {
                        s.session.recording_active = active;
                        s.session.recording_text = text;
                    }
                }
            }
        }

        self.poll_hotplug();

        if let Some(tab) = self.sessions.get(self.active) {
            if tab.kind == SessionKind::Terminal && now >= tab.last_blink {
                let s = &mut self.sessions[self.active];
                s.last_blink = now + Duration::from_millis(530);
                s.cursor_on = !s.cursor_on;
                s.term_dirty = true;
            }
        }

        if self.last_status.elapsed() >= Duration::from_millis(100) {
            self.last_status = Instant::now();
            let log_dirty = self.sessions[self.active].log_dirty;
            if log_dirty {
                self.sessions[self.active].log_dirty = false;
                self.push_log();
            }
            if self.sessions[self.active].term_dirty {
                self.sessions[self.active].term_dirty = false;
                self.push_term();
            }
            self.push_status();
        }

        if self.cfg_dirty && self.last_save.elapsed() >= Duration::from_millis(600) {
            self.cfg.save();
            self.cfg_dirty = false;
            self.last_save = Instant::now();
        }
    }

    fn poll_hotplug(&mut self) {
        if let Ok(devices) = self.scan_rx.try_recv() {
            self.scanning = false;
            self.next_scan = Instant::now() + Duration::from_secs(2);
            let signature: Vec<String> = devices.iter().map(|d| d.path.clone()).collect();
            if signature != self.signature {
                self.signature = signature;
                self.devices = devices;
                self.push_devices();
                for s in &self.sessions {
                    if s.session.state == ConnectionState::Reconnecting {
                        s.session.send(Command::ProbeNow);
                    }
                }
            }
        }
        if self.scanning || Instant::now() < self.next_scan {
            return;
        }
        self.scanning = true;
        let tx = self.scan_tx.clone();
        std::thread::spawn(move || {
            let devices = scan_devices();
            let _ = tx.send(devices);
        });
    }

    pub fn set_window_size(&mut self, width: f32, height: f32) {
        if width >= 960.0 && height >= 600.0 {
            self.cfg.window_width = width;
            self.cfg.window_height = height;
        }
    }

    pub fn shutdown(&mut self) {
        for s in &mut self.sessions {
            s.session.shutdown();
        }
        self.cfg.save();
    }
}

fn line_json(line: &LogLine) -> Value {
    let segs: Vec<Value> = line
        .segments
        .iter()
        .map(|s| {
            let role = match s.role {
                LogRole::Text => 0,
                LogRole::Muted => 1,
                LogRole::Tx => 2,
                LogRole::Hex => 3,
                LogRole::Highlight => 4,
            };
            json!([s.text, role, if s.bold { 1 } else { 0 }])
        })
        .collect();
    Value::Array(segs)
}
