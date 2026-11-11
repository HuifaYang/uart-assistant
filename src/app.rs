use crate::{AppWindow, MacroDialog, TabItem, UiSegment};
use crate::config::{Config, MacroItem};
use crate::monitor::{Monitor, ENCODINGS};
use crate::port::scan_devices;
use crate::session::Session;
use crate::terminal::TerminalEmulator;
use crate::types::*;
use encoding_rs::{Decoder, Encoding};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel, Weak};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

const BAUDS: [u32; 8] = [9600, 19200, 38400, 57600, 115200, 230400, 460800, 921600];

pub struct SessionUi {
    pub session: Session,
    pub monitor: Monitor,
    pub terminal: TerminalEmulator,
    term_decoder: Decoder,
    pub title: String,
    pub settings: PortSettings,
    pub history: VecDeque<String>,
    pub loop_enabled: bool,
    pub loop_interval_ms: u64,
    pub show_monitor: bool,
    pub xmodem_path: Option<PathBuf>,
    pub use_1k: bool,
    pub search: String,
    pub log_dirty: bool,
    pub term_dirty: bool,
    pub transfer_done: u64,
    pub transfer_total: u64,
    hex_send: bool,
    append_newline: bool,
}

fn decoder_for(index: usize) -> Decoder {
    Encoding::for_label(ENCODINGS[index.min(ENCODINGS.len() - 1)].1.as_bytes())
        .unwrap()
        .new_decoder()
}

fn to_ui_segments(segments: &[Segment]) -> ModelRc<UiSegment> {
    let items: Vec<UiSegment> = segments
        .iter()
        .map(|s| UiSegment {
            text: SharedString::from(s.text.as_str()),
            fg: Color::from_rgb_u8(s.fg.0, s.fg.1, s.fg.2),
            bold: s.bold,
        })
        .collect();
    ModelRc::new(VecModel::from(items))
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

fn stamp_now() -> String {
    chrono::Local::now().format("%H:%M:%S.%3f").to_string()
}

pub struct AppState {
    ui: Weak<AppWindow>,
    sessions: Vec<SessionUi>,
    next_id: usize,
    active: usize,
    devices: Vec<DeviceInfo>,
    signature: Vec<String>,
    next_scan: Instant,
    config: Config,
    log_rows: Rc<VecModel<ModelRc<UiSegment>>>,
    term_rows: Rc<VecModel<ModelRc<UiSegment>>>,
    macro_dialog: Option<MacroDialog>,
    config_dirty: bool,
}

impl AppState {
    pub fn new(ui: Weak<AppWindow>) -> Self {
        let config = Config::load();
        let devices = scan_devices();
        let signature = devices.iter().map(|d| d.path.clone()).collect();
        let log_rows = Rc::new(VecModel::from(Vec::<ModelRc<UiSegment>>::new()));
        let term_rows = Rc::new(VecModel::from(Vec::<ModelRc<UiSegment>>::new()));
        if let Some(window) = ui.upgrade() {
            window.set_log_lines(ModelRc::from(log_rows.clone()));
            window.set_terminal_lines(ModelRc::from(term_rows.clone()));
        }
        let mut state = Self {
            ui,
            sessions: Vec::new(),
            next_id: 0,
            active: 0,
            devices,
            signature,
            next_scan: Instant::now() + Duration::from_millis(1500),
            config,
            log_rows,
            term_rows,
            macro_dialog: None,
            config_dirty: false,
        };
        state.add_session();
        state
    }

    fn add_session(&mut self) {
        let id = self.next_id;
        self.next_id += 1;
        let settings = self.config.last_settings.clone();
        let mut monitor = Monitor::new();
        monitor.timestamps = self.config.monitor.timestamps;
        monitor.autoscroll = self.config.monitor.autoscroll;
        monitor.echo_tx = self.config.monitor.echo_tx;
        monitor.highlight = self.config.monitor.highlight.clone();
        monitor.line_limit = self.config.monitor.line_limit.max(200) as usize;
        let encoding_index = self.config.monitor.encoding_index.max(0) as usize;
        monitor.set_encoding(encoding_index.min(ENCODINGS.len() - 1));
        monitor.set_mode(DisplayMode::from_index(self.config.monitor.mode));
        let session = Session::new(id, settings.clone(), self.config.auto_reconnect);
        self.sessions.push(SessionUi {
            session,
            monitor,
            terminal: TerminalEmulator::new(100, 30),
            term_decoder: decoder_for(encoding_index),
            title: format!("会话 {}", id + 1),
            settings,
            history: VecDeque::new(),
            loop_enabled: false,
            loop_interval_ms: self.config.monitor.loop_interval_ms.max(50) as u64,
            show_monitor: true,
            xmodem_path: None,
            use_1k: false,
            search: String::new(),
            log_dirty: true,
            term_dirty: true,
            transfer_done: 0,
            transfer_total: 0,
            hex_send: self.config.monitor.hex_send,
            append_newline: self.config.monitor.append_newline,
        });
        self.active = self.sessions.len() - 1;
    }

    pub fn sync_to_ui(&mut self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let names: Vec<SharedString> = self
            .devices
            .iter()
            .map(|d| SharedString::from(format!("{} · {}", d.label, d.detail)))
            .collect();
        ui.set_device_names(ModelRc::new(VecModel::from(names)));
        let port = self.sessions[self.active].settings.port.clone();
        let index = self
            .devices
            .iter()
            .position(|d| d.path == port)
            .map(|i| i as i32)
            .unwrap_or(-1);
        ui.set_device_index(index);
        ui.set_custom_port(SharedString::from(port));
        let baud = self.sessions[self.active].settings.baud;
        ui.set_baud_index(BAUDS.iter().position(|b| *b == baud).unwrap_or(4) as i32);
        let settings = &self.sessions[self.active].settings;
        ui.set_data_bits_index((settings.data_bits.saturating_sub(5) as i32).min(3));
        ui.set_stop_bits_index((settings.stop_bits.saturating_sub(1) as i32).min(1));
        ui.set_parity_index(settings.parity.index());
        ui.set_flow_index(settings.flow.index());

        let s = &self.sessions[self.active];
        ui.set_timestamps(s.monitor.timestamps);
        ui.set_autoscroll(s.monitor.autoscroll);
        ui.set_paused(s.monitor.paused);
        ui.set_echo_tx(s.monitor.echo_tx);
        ui.set_hex_send(s.hex_send);
        ui.set_append_newline(s.append_newline);
        ui.set_loop_send(s.loop_enabled);
        ui.set_loop_interval(s.loop_interval_ms as i32);
        ui.set_display_mode(s.monitor.mode.index());
        ui.set_encoding_index(s.monitor.encoding_index as i32);
        ui.set_line_limit(s.monitor.line_limit as i32);
        ui.set_highlight(SharedString::from(s.monitor.highlight.clone()));
        ui.set_search_text(SharedString::from(s.search.clone()));
        ui.set_show_monitor(s.show_monitor);
        ui.set_use_1k(s.use_1k);
        ui.set_auto_reconnect(self.config.auto_reconnect);
        ui.set_xmodem_file(SharedString::from(
            s.xmodem_path
                .as_ref()
                .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned())
                .unwrap_or_else(|| "未选择文件".into()),
        ));

        let tabs: Vec<TabItem> = self
            .sessions
            .iter()
            .map(|item| TabItem {
                title: SharedString::from(item.title.as_str()),
                connected: item.session.state.connected(),
            })
            .collect();
        ui.set_tabs(ModelRc::new(VecModel::from(tabs)));
        ui.set_active_tab(self.active as i32);
        let macros: Vec<SharedString> = self
            .config
            .macros
            .iter()
            .map(|m| SharedString::from(m.name.as_str()))
            .collect();
        ui.set_macro_names(ModelRc::new(VecModel::from(macros)));
        self.refresh_log_model();
        self.refresh_term_model();
        self.sync_status(&ui);
    }

    fn current_settings(&self, ui: &AppWindow) -> PortSettings {
        let custom = ui.get_custom_port().to_string();
        let port = if !custom.trim().is_empty() {
            custom.trim().to_string()
        } else {
            let index = ui.get_device_index();
            if index >= 0 {
                self.devices
                    .get(index as usize)
                    .map(|d| d.path.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            }
        };
        PortSettings {
            port,
            baud: BAUDS.get(ui.get_baud_index().max(0) as usize).copied().unwrap_or(115200),
            data_bits: (ui.get_data_bits_index().clamp(0, 3) + 5) as u8,
            stop_bits: (ui.get_stop_bits_index().clamp(0, 1) + 1) as u8,
            parity: Parity::from_index(ui.get_parity_index()),
            flow: FlowControl::from_index(ui.get_flow_index()),
        }
    }

    fn sync_from_ui(&mut self, ui: &AppWindow) {
        let idx = self.active;
        let s = &mut self.sessions[idx];
        let mut changed = false;

        macro_rules! flag {
            ($getter:ident, $field:ident) => {{
                let value = ui.$getter();
                if s.monitor.$field != value {
                    s.monitor.$field = value;
                    changed = true;
                }
            }};
        }
        flag!(get_timestamps, timestamps);
        flag!(get_autoscroll, autoscroll);
        flag!(get_paused, paused);
        flag!(get_echo_tx, echo_tx);

        let highlight = ui.get_highlight().to_string();
        if s.monitor.highlight != highlight {
            s.monitor.highlight = highlight;
            changed = true;
        }
        let search = ui.get_search_text().to_string();
        if s.search != search {
            s.search = search;
            changed = true;
        }
        let limit = ui.get_line_limit().max(200) as usize;
        if s.monitor.line_limit != limit {
            s.monitor.set_line_limit(limit);
            changed = true;
        }
        let mode = DisplayMode::from_index(ui.get_display_mode());
        if s.monitor.mode != mode {
            s.monitor.set_mode(mode);
            changed = true;
        }
        let encoding = (ui.get_encoding_index().max(0) as usize).min(ENCODINGS.len() - 1);
        if s.monitor.encoding_index != encoding {
            s.monitor.set_encoding(encoding);
            s.term_decoder = decoder_for(encoding);
            s.term_dirty = true;
        }
        let hex_send = ui.get_hex_send();
        let append_newline = ui.get_append_newline();
        let interval = ui.get_loop_interval().max(50) as u64;
        let loop_enabled = ui.get_loop_send();
        let loop_changed = interval != s.loop_interval_ms || loop_enabled != s.loop_enabled;
        s.hex_send = hex_send;
        s.append_newline = append_newline;
        s.loop_interval_ms = interval;
        s.loop_enabled = loop_enabled;
        s.show_monitor = ui.get_show_monitor();
        s.use_1k = ui.get_use_1k();

        if loop_changed {
            let text = if ui.get_multi_line() && !ui.get_multi_send_text().is_empty() {
                ui.get_multi_send_text().to_string()
            } else {
                ui.get_send_text().to_string()
            };
            if let Ok(data) = parse_send(&text, hex_send, append_newline) {
                s.session.send(Command::ConfigureLoop {
                    data,
                    interval_ms: interval,
                    enabled: loop_enabled,
                });
            }
        }

        if changed {
            s.log_dirty = true;
            self.config.monitor.timestamps = s.monitor.timestamps;
            self.config.monitor.autoscroll = s.monitor.autoscroll;
            self.config.monitor.echo_tx = s.monitor.echo_tx;
            self.config.monitor.highlight = s.monitor.highlight.clone();
            self.config.monitor.line_limit = s.monitor.line_limit as i32;
            self.config.monitor.mode = s.monitor.mode.index();
            self.config.monitor.encoding_index = s.monitor.encoding_index as i32;
            self.config.monitor.loop_interval_ms = interval as i32;
            self.config.monitor.hex_send = hex_send;
            self.config.monitor.append_newline = append_newline;
            self.config_dirty = true;
        }

        let auto_reconnect = ui.get_auto_reconnect();
        if auto_reconnect != self.config.auto_reconnect {
            self.config.auto_reconnect = auto_reconnect;
            self.sessions[idx].session.send(Command::SetAutoReconnect(auto_reconnect));
            self.config_dirty = true;
        }
    }

    fn refresh_log_model(&self) {
        let s = &self.sessions[self.active];
        let lines = s.monitor.visible_lines(&s.search);
        let keyword = if !s.search.is_empty() {
            s.search.clone()
        } else {
            s.monitor.highlight.clone()
        };
        let base = match s.monitor.mode {
            DisplayMode::Text => color::TEXT,
            DisplayMode::Hex | DisplayMode::AsciiHex => color::TEAL,
        };
        let rows: Vec<ModelRc<UiSegment>> = lines
            .iter()
            .map(|line| {
                let mut segments = Vec::new();
                let mut rest = line.text.as_str();
                if s.monitor.timestamps && rest.starts_with('[') {
                    if let Some(end) = rest.find("] ") {
                        segments.push(Segment::plain(rest[..end + 2].to_string(), color::MUTED));
                        rest = &rest[end + 2..];
                    }
                }
                let fg = if rest.starts_with("TX> ") {
                    color::BLUE
                } else {
                    base
                };
                segments.extend(split_highlight(rest, &keyword, fg));
                to_ui_segments(&segments)
            })
            .collect();
        self.log_rows.set_vec(rows);
    }

    fn refresh_term_model(&self) {
        let s = &self.sessions[self.active];
        let rows: Vec<ModelRc<UiSegment>> = s
            .terminal
            .render_lines()
            .iter()
            .map(|segments| to_ui_segments(segments))
            .collect();
        self.term_rows.set_vec(rows);
    }

    fn sync_status(&self, ui: &AppWindow) {
        let s = &self.sessions[self.active];
        ui.set_connection_label(SharedString::from(s.session.state.label()));
        ui.set_connected(s.session.state.connected());
        ui.set_rx_label(SharedString::from(format!("RX {}", s.session.rx_bytes)));
        ui.set_tx_label(SharedString::from(format!("TX {}", s.session.tx_bytes)));
        ui.set_device_label(SharedString::from(format!("设备 {}", self.devices.len())));
        ui.set_status_label(SharedString::from(s.session.last_message.clone()));
        ui.set_log_summary(SharedString::from(format!("{} 行", s.monitor.line_count())));
        ui.set_record_active(s.session.recording_active);
        ui.set_record_label(SharedString::from(if s.session.recording_active {
            s.session.recording_text.clone()
        } else {
            "原始记录 关".into()
        }));
        ui.set_transfer_active(s.session.transferring);
        ui.set_transfer_label(SharedString::from(s.session.transfer_text.clone()));
        let progress = if s.transfer_total > 0 {
            (s.transfer_done as f32 / s.transfer_total as f32).min(1.0)
        } else {
            0.0
        };
        ui.set_transfer_progress(progress);
    }

    fn poll_hotplug(&mut self, ui: &AppWindow) {
        if Instant::now() < self.next_scan {
            return;
        }
        self.next_scan = Instant::now() + Duration::from_millis(1500);
        let devices = scan_devices();
        let signature: Vec<String> = devices.iter().map(|d| d.path.clone()).collect();
        if signature == self.signature {
            return;
        }
        self.signature = signature;
        self.devices = devices;
        let names: Vec<SharedString> = self
            .devices
            .iter()
            .map(|d| SharedString::from(format!("{} · {}", d.label, d.detail)))
            .collect();
        ui.set_device_names(ModelRc::new(VecModel::from(names)));
        for s in &self.sessions {
            if s.session.state == ConnectionState::Reconnecting {
                s.session.send(Command::ProbeNow);
            }
        }
    }

    pub fn tick(&mut self) {
        let stamp = stamp_now();
        let mut needs_log = false;
        let mut needs_term = false;
        for s in self.sessions.iter_mut() {
            for event in s.session.drain_events() {
                match event {
                    SessionEvent::Data(bytes) => {
                        s.monitor.feed_rx(&bytes, &stamp);
                        let mut decoded = String::new();
                        let _ = s.term_decoder.decode_to_string(&bytes, &mut decoded, false);
                        s.terminal.feed(&decoded);
                        needs_log = true;
                        needs_term = true;
                    }
                    SessionEvent::State(state) => s.session.state = state,
                    SessionEvent::Stats { rx, tx } => {
                        s.session.rx_bytes = rx;
                        s.session.tx_bytes = tx;
                    }
                    SessionEvent::Message(message) => s.session.last_message = message,
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
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        self.sync_from_ui(&ui);
        let active = self.active;
        let log_dirty = self.sessions[active].log_dirty;
        let term_dirty = self.sessions[active].term_dirty;
        if needs_log || log_dirty {
            self.refresh_log_model();
            self.sessions[active].log_dirty = false;
            if self.sessions[active].monitor.autoscroll {
                ui.invoke_scroll_to_bottom();
            }
        }
        if needs_term || term_dirty {
            self.refresh_term_model();
            self.sessions[active].term_dirty = false;
        }
        self.sync_status(&ui);
        self.poll_hotplug(&ui);
        if self.config_dirty {
            self.config.save();
            self.config_dirty = false;
        }
    }

    fn new_tab(&mut self) {
        self.add_session();
        self.sync_to_ui();
    }

    fn close_tab(&mut self, index: i32) {
        if self.sessions.len() <= 1 || index < 0 {
            return;
        }
        let index = index as usize;
        if index >= self.sessions.len() {
            return;
        }
        let mut removed = self.sessions.remove(index);
        removed.session.shutdown();
        self.active = self.active.min(self.sessions.len() - 1);
        self.sessions[self.active].log_dirty = true;
        self.sessions[self.active].term_dirty = true;
        self.sync_to_ui();
    }

    fn select_tab(&mut self, index: i32) {
        if index < 0 || index as usize >= self.sessions.len() {
            return;
        }
        self.active = index as usize;
        self.sessions[self.active].log_dirty = true;
        self.sessions[self.active].term_dirty = true;
        self.sync_to_ui();
    }

    fn toggle_connect(&mut self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let settings = self.current_settings(&ui);
        let s = &mut self.sessions[self.active];
        if s.session.state.connected() || s.session.state == ConnectionState::Opening {
            s.session.send(Command::Close);
        } else {
            if settings.port.is_empty() {
                s.session.last_message = "请选择或输入串口设备".into();
                return;
            }
            s.settings = settings.clone();
            s.session.send(Command::SetAutoReconnect(self.config.auto_reconnect));
            s.session.send(Command::Open(settings.clone()));
            self.config.remember(&settings.port, &settings);
            self.config_dirty = true;
            ui.set_custom_port(SharedString::from(settings.port));
        }
    }

    fn send_current(&mut self, newline: bool, hex_override: Option<bool>) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let text = if ui.get_multi_line() && !ui.get_multi_send_text().is_empty() {
            ui.get_multi_send_text().to_string()
        } else {
            ui.get_send_text().to_string()
        };
        if text.is_empty() {
            return;
        }
        let hex = hex_override.unwrap_or_else(|| ui.get_hex_send());
        let append = newline && self.sessions[self.active].append_newline && !hex;
        let bytes = match parse_send(&text, hex, append) {
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
            s.monitor.feed_tx(&text, &stamp_now());
            s.log_dirty = true;
        }
        s.history.push_back(text);
        while s.history.len() > 30 {
            s.history.pop_front();
        }
    }

    fn send_macro(&mut self, index: i32) {
        let Some(macro_item) = self.config.macros.get(index.max(0) as usize).cloned() else {
            return;
        };
        let bytes = match parse_send(&macro_item.command, macro_item.hex, false) {
            Ok(bytes) => bytes,
            Err(err) => {
                self.sessions[self.active].session.last_message = err;
                return;
            }
        };
        let s = &mut self.sessions[self.active];
        s.session.send(Command::Send(bytes));
        if s.monitor.echo_tx {
            s.monitor.feed_tx(&macro_item.command, &stamp_now());
            s.log_dirty = true;
        }
    }

    fn macros_text(&self) -> String {
        self.config
            .macros
            .iter()
            .map(|m| {
                let escaped = m.command.replace('\r', "\\r").replace('\n', "\\n");
                format!("{} | {} | {}", m.name, escaped, if m.hex { "1" } else { "0" })
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn apply_macros(&mut self, text: SharedString) {
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
                hex: parts.get(2).map(|v| v.trim() == "1").unwrap_or(false),
            });
        }
        self.config.macros = macros;
        self.config_dirty = true;
        self.sync_to_ui();
    }

    pub fn hide_macro_dialog(&self) {
        if let Some(dialog) = &self.macro_dialog {
            let _ = dialog.hide();
        }
    }

    pub fn shutdown(&mut self) {
        for s in &mut self.sessions {
            s.session.shutdown();
        }
        self.config.save();
    }
}

pub fn bind(ui: &AppWindow, state: &Rc<RefCell<AppState>>, dialog: MacroDialog) {
    {
        let s = state.clone();
        ui.on_new_tab(move || s.borrow_mut().new_tab());
    }
    {
        let s = state.clone();
        ui.on_close_tab(move |i| s.borrow_mut().close_tab(i));
    }
    {
        let s = state.clone();
        ui.on_select_tab(move |i| s.borrow_mut().select_tab(i));
    }
    {
        let s = state.clone();
        ui.on_rename_save(move |i, text| {
            if i >= 0 {
                let mut app = s.borrow_mut();
                if let Some(item) = app.sessions.get_mut(i as usize) {
                    item.title = text.to_string();
                }
                app.sync_to_ui();
            }
        });
    }
    {
        let s = state.clone();
        ui.on_toggle_connect(move || s.borrow_mut().toggle_connect());
    }
    {
        let s = state.clone();
        ui.on_rescan(move || {
            let mut app = s.borrow_mut();
            app.next_scan = Instant::now() - Duration::from_secs(1);
            app.signature.clear();
        });
    }
    {
        let s = state.clone();
        ui.on_device_changed(move |i| {
            let mut app = s.borrow_mut();
            let port = if i >= 0 {
                app.devices.get(i as usize).map(|d| d.path.clone())
            } else {
                None
            };
            if let Some(port) = port {
                if let Some(saved) = app.config.profile_for(&port) {
                    let state_baud = saved.baud;
                    if let Some(ui) = app.ui.upgrade() {
                        ui.set_baud_index(BAUDS.iter().position(|b| *b == state_baud).unwrap_or(4) as i32);
                        ui.set_data_bits_index((saved.data_bits.saturating_sub(5) as i32).min(3));
                        ui.set_stop_bits_index((saved.stop_bits.saturating_sub(1) as i32).min(1));
                        ui.set_parity_index(saved.parity.index());
                        ui.set_flow_index(saved.flow.index());
                        ui.set_custom_port(SharedString::from(port));
                    }
                }
            }
        });
    }
    {
        let s = state.clone();
        ui.on_send(move || s.borrow_mut().send_current(true, None));
    }
    {
        let s = state.clone();
        ui.on_clear_log(move || {
            let mut app = s.borrow_mut();
            let index = app.active;
            app.sessions[index].monitor.clear();
            app.sessions[index].log_dirty = true;
        });
    }
    {
        let s = state.clone();
        ui.on_save_log(move || {
            let app = s.borrow();
            let text = app.sessions[app.active].monitor.plain_text();
            if let Some(path) = rfd::FileDialog::new().set_file_name("uart-log.txt").save_file() {
                let _ = std::fs::write(path, text);
            }
        });
    }
    {
        let s = state.clone();
        ui.on_toggle_record(move || {
            let mut app = s.borrow_mut();
            let index = app.active;
            if app.sessions[index].session.recording_active {
                app.sessions[index].session.send(Command::StopRecording);
            } else if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                app.sessions[index].session.send(Command::StartRecording(dir));
            }
        });
    }
    {
        let s = state.clone();
        ui.on_set_display_mode(move |mode| {
            let mut app = s.borrow_mut();
            if let Some(ui) = app.ui.upgrade() {
                ui.set_display_mode(mode);
            }
            let index = app.active;
            app.sessions[index].log_dirty = true;
        });
    }
    {
        let s = state.clone();
        ui.on_refresh_monitor(move || {
            let mut app = s.borrow_mut();
            let index = app.active;
            app.sessions[index].log_dirty = true;
        });
    }
    {
        let s = state.clone();
        ui.on_dtr_changed(move |value| s.borrow().sessions[s.borrow().active].session.send(Command::SetDtr(value)));
    }
    {
        let s = state.clone();
        ui.on_rts_changed(move |value| s.borrow().sessions[s.borrow().active].session.send(Command::SetRts(value)));
    }
    {
        let s = state.clone();
        ui.on_stm32(move |mode| {
            let action = if mode == 1 { Stm32Action::Bootloader } else { Stm32Action::Reset };
            s.borrow().sessions[s.borrow().active].session.send(Command::Stm32(action));
        });
    }
    {
        let s = state.clone();
        ui.on_choose_xmodem(move || {
            if let Some(path) = rfd::FileDialog::new().pick_file() {
                let mut app = s.borrow_mut();
                let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let index = app.active;
                app.sessions[index].xmodem_path = Some(path);
                if let Some(ui) = app.ui.upgrade() {
                    ui.set_xmodem_file(SharedString::from(name));
                }
            }
        });
    }
    {
        let s = state.clone();
        ui.on_start_xmodem(move || {
            let mut app = s.borrow_mut();
            let index = app.active;
            let Some(path) = app.sessions[index].xmodem_path.clone() else {
                app.sessions[index].session.last_message = "请先选择 XModem 文件".into();
                return;
            };
            let use_1k = app.sessions[index].use_1k;
            app.sessions[index].session.send(Command::StartXmodem { path, use_1k });
        });
    }
    {
        let s = state.clone();
        ui.on_cancel_transfer(move || {
            s.borrow().sessions[s.borrow().active].session.send(Command::CancelTransfer);
        });
    }
    {
        let s = state.clone();
        let dialog_weak = dialog.as_weak();
        ui.on_edit_macros(move || {
            let app = s.borrow();
            let text = app.macros_text();
            if let Some(dialog) = dialog_weak.upgrade() {
                dialog.set_content(SharedString::from(text));
                let _ = dialog.show();
            }
        });
    }
    {
        let s = state.clone();
        ui.on_send_macro(move |i| s.borrow_mut().send_macro(i));
    }
    {
        let s = state.clone();
        ui.on_toggle_loop(move |_enabled| {
            let mut app = s.borrow_mut();
            let index = app.active;
            app.sessions[index].log_dirty = true;
        });
    }
    {
        let s = state.clone();
        ui.on_toggle_terminal(move |terminal| {
            let mut app = s.borrow_mut();
            if let Some(ui) = app.ui.upgrade() {
                ui.set_show_monitor(!terminal);
            }
        });
    }
    {
        let s = state.clone();
        ui.on_terminal_key(move |text| {
            let value = text.to_string();
            s.borrow_mut().send_terminal_key(value);
        });
    }
    {
        let s = state.clone();
        dialog.on_save(move |text| {
            s.borrow_mut().apply_macros(text);
            s.borrow().hide_macro_dialog();
        });
    }
    {
        let s = state.clone();
        dialog.on_cancel(move || s.borrow().hide_macro_dialog());
    }
    state.borrow_mut().macro_dialog = Some(dialog);
}

impl AppState {
    fn send_terminal_key(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        let mut bytes = text.into_bytes();
        bytes.push(b'\r');
        self.sessions[self.active].session.send(Command::Send(bytes));
        if let Some(ui) = self.ui.upgrade() {
            ui.set_send_text(SharedString::from(""));
        }
    }
}
