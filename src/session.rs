use crate::port::{open_port, port_exists, set_dtr, set_rts, stm32_sequence};
use crate::raw_log::RawLogWriter;
use crate::types::{Command, ConnectionState, PortSettings, SessionEvent};
use crate::xmodem;
use serialport::SerialPort;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const BASE_RECONNECT_MS: u64 = 1500;
const MAX_RECONNECT_MS: u64 = 30000;

struct Reconnect {
    next: Instant,
    interval_ms: u64,
}

struct Worker {
    port: Option<Box<dyn SerialPort>>,
    settings: PortSettings,
    want_connected: bool,
    auto_reconnect: bool,
    reconnect: Option<Reconnect>,
    loop_data: Vec<u8>,
    loop_interval_ms: u64,
    loop_enabled: bool,
    next_loop: Instant,
    recorder: Option<RawLogWriter>,
    last_flush: Instant,
    cancel: Arc<AtomicBool>,
    rx: u64,
    tx: u64,
}

impl Worker {
    fn emit(&self, tx: &mpsc::Sender<SessionEvent>, event: SessionEvent) {
        let _ = tx.send(event);
    }

    fn stats(&self, tx: &mpsc::Sender<SessionEvent>) {
        self.emit(tx, SessionEvent::Stats { rx: self.rx, tx: self.tx });
    }

    fn start_reconnect(&mut self, tx: &mpsc::Sender<SessionEvent>) {
        self.reconnect = Some(Reconnect {
            next: Instant::now(),
            interval_ms: BASE_RECONNECT_MS,
        });
        self.emit(tx, SessionEvent::State(ConnectionState::Reconnecting));
    }

    fn open(&mut self, tx: &mpsc::Sender<SessionEvent>) {
        self.emit(tx, SessionEvent::State(ConnectionState::Opening));
        match open_port(&self.settings) {
            Ok(port) => {
                self.port = Some(port);
                self.reconnect = None;
                self.emit(tx, SessionEvent::State(ConnectionState::Connected));
                self.emit(
                    tx,
                    SessionEvent::Message(format!("已连接 {} @ {}", self.settings.port, self.settings.baud)),
                );
            }
            Err(err) => {
                self.emit(tx, SessionEvent::State(ConnectionState::Disconnected));
                self.emit(tx, SessionEvent::Message(format!("打开失败: {}", err)));
                if self.auto_reconnect {
                    self.start_reconnect(tx);
                }
            }
        }
    }

    fn handle(&mut self, cmd: Command, tx: &mpsc::Sender<SessionEvent>) -> bool {
        match cmd {
            Command::Open(settings) => {
                self.settings = settings;
                self.want_connected = true;
                self.cancel.store(false, Ordering::Relaxed);
                self.open(tx);
            }
            Command::Close | Command::StopReconnect => {
                self.want_connected = false;
                self.reconnect = None;
                self.port.take();
                self.loop_enabled = false;
                self.cancel.store(true, Ordering::Relaxed);
                self.emit(tx, SessionEvent::State(ConnectionState::Disconnected));
                self.emit(tx, SessionEvent::Message(if matches!(cmd, Command::Close) { "已断开" } else { "已停止重连" }.into()));
            }
            Command::Send(data) => {
                if let Some(port) = self.port.as_mut() {
                    match port.write_all(&data) {
                        Ok(()) => {
                            let _ = port.flush();
                            self.tx += data.len() as u64;
                            self.stats(tx);
                        }
                        Err(err) => self.emit(tx, SessionEvent::Message(format!("发送失败: {}", err))),
                    }
                }
            }
            Command::ConfigureLoop { data, interval_ms, enabled } => {
                self.loop_data = data;
                self.loop_interval_ms = interval_ms.max(50);
                self.loop_enabled = enabled;
                self.next_loop = Instant::now() + Duration::from_millis(self.loop_interval_ms);
            }
            Command::SetDtr(level) => {
                if let Some(port) = self.port.as_mut() {
                    if let Err(err) = set_dtr(port.as_mut(), level) {
                        self.emit(tx, SessionEvent::Message(format!("DTR 设置失败: {}", err)));
                    }
                }
            }
            Command::SetRts(level) => {
                if let Some(port) = self.port.as_mut() {
                    if let Err(err) = set_rts(port.as_mut(), level) {
                        self.emit(tx, SessionEvent::Message(format!("RTS 设置失败: {}", err)));
                    }
                }
            }
            Command::Stm32(action) => {
                if let Some(port) = self.port.as_mut() {
                    let bootloader = matches!(action, crate::types::Stm32Action::Bootloader);
                    if let Err(err) = stm32_sequence(port.as_mut(), bootloader) {
                        self.emit(tx, SessionEvent::Message(format!("MCU 时序失败: {}", err)));
                    } else {
                        self.emit(tx, SessionEvent::Message(if bootloader { "已进入 Bootloader 时序" } else { "已执行复位时序" }.into()));
                    }
                }
            }
            Command::StartXmodem { path, use_1k } => {
                self.loop_enabled = false;
                let Some(mut port) = self.port.take() else {
                    self.emit(tx, SessionEvent::Message("请先打开串口".into()));
                    return true;
                };
                let data = match std::fs::read(&path) {
                    Ok(data) => data,
                    Err(err) => {
                        self.port = Some(port);
                        self.emit(tx, SessionEvent::Message(format!("读取文件失败: {}", err)));
                        return true;
                    }
                };
                let total = data.len() as u64;
                self.emit(tx, SessionEvent::Transfer { active: true, done: 0, total, message: "等待接收端…".into() });
                self.cancel.store(false, Ordering::Relaxed);
                let cancel = self.cancel.clone();
                let mut on_progress = |done: u64, total: u64| {
                    let _ = tx.send(SessionEvent::Transfer {
                        active: true,
                        done,
                        total,
                        message: format!("{}/{} 字节", done, total),
                    });
                };
                let result = xmodem::send_file(port.as_mut(), &data, use_1k, &cancel, &mut on_progress);
                self.port = Some(port);
                let message = match result {
                    Ok(()) => "XModem 发送完成".to_string(),
                    Err(err) => format!("XModem: {}", err),
                };
                self.emit(tx, SessionEvent::Transfer { active: false, done: 0, total, message: message.clone() });
                self.emit(tx, SessionEvent::Message(message));
            }
            Command::CancelTransfer => {
                self.cancel.store(true, Ordering::Relaxed);
            }
            Command::StartRecording(dir) => {
                match RawLogWriter::start(dir) {
                    Ok(writer) => {
                        let text = writer.description();
                        self.recorder = Some(writer);
                        self.emit(tx, SessionEvent::Recording { active: true, text });
                    }
                    Err(err) => self.emit(tx, SessionEvent::Message(err)),
                }
            }
            Command::StopRecording => {
                if let Some(mut writer) = self.recorder.take() {
                    writer.stop();
                }
                self.emit(tx, SessionEvent::Recording { active: false, text: "原始记录已停止".into() });
            }
            Command::ProbeNow => {
                if let Some(r) = self.reconnect.as_mut() {
                    r.next = Instant::now();
                }
            }
            Command::SetAutoReconnect(value) => {
                self.auto_reconnect = value;
                if !value {
                    self.reconnect = None;
                }
            }
            Command::Shutdown => {
                self.want_connected = false;
                self.cancel.store(true, Ordering::Relaxed);
                self.port.take();
                if let Some(mut writer) = self.recorder.take() {
                    writer.stop();
                }
                return false;
            }
        }
        true
    }

    fn poll_port(&mut self, tx: &mpsc::Sender<SessionEvent>) {
        let Some(port) = self.port.as_mut() else {
            return;
        };
        let available = port.bytes_to_read().unwrap_or(0) as usize;
        if available == 0 {
            return;
        }
        let mut buf = vec![0u8; available.min(32768)];
        match port.read(&mut buf) {
            Ok(n) if n > 0 => {
                buf.truncate(n);
                self.rx += n as u64;
                if let Some(writer) = self.recorder.as_mut() {
                    if let Err(err) = writer.write(&buf) {
                        writer.stop();
                        self.recorder.take();
                        self.emit(tx, SessionEvent::Recording { active: false, text: err });
                    }
                }
                self.emit(tx, SessionEvent::Data(buf));
                self.stats(tx);
            }
            Err(err) => {
                let transient = matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock);
                if !transient {
                    self.emit(tx, SessionEvent::Message(format!("串口读取中断: {}", err)));
                    self.port.take();
                    self.loop_enabled = false;
                    if self.want_connected && self.auto_reconnect {
                        self.start_reconnect(tx);
                    } else {
                        self.emit(tx, SessionEvent::State(ConnectionState::Disconnected));
                    }
                }
            }
            _ => {}
        }
    }

    fn poll_reconnect(&mut self, tx: &mpsc::Sender<SessionEvent>) {
        let Some(mut reconnect) = self.reconnect.take() else {
            return;
        };
        if !self.want_connected || !self.auto_reconnect {
            return;
        }
        if Instant::now() < reconnect.next {
            self.reconnect = Some(reconnect);
            return;
        }
        if port_exists(&self.settings.port) {
            match open_port(&self.settings) {
                Ok(port) => {
                    self.port = Some(port);
                    self.emit(tx, SessionEvent::State(ConnectionState::Connected));
                    self.emit(tx, SessionEvent::Message("重连成功".into()));
                    return;
                }
                Err(err) => self.emit(tx, SessionEvent::Message(format!("重连失败: {}", err))),
            }
        }
        reconnect.interval_ms = (reconnect.interval_ms * 2).min(MAX_RECONNECT_MS);
        reconnect.next = Instant::now() + Duration::from_millis(reconnect.interval_ms);
        self.reconnect = Some(reconnect);
    }

    fn poll_loop(&mut self, tx: &mpsc::Sender<SessionEvent>) {
        if !self.loop_enabled || self.loop_data.is_empty() || self.port.is_none() {
            return;
        }
        if Instant::now() < self.next_loop {
            return;
        }
        self.next_loop = Instant::now() + Duration::from_millis(self.loop_interval_ms);
        let data = self.loop_data.clone();
        if let Some(port) = self.port.as_mut() {
            if port.write_all(&data).is_ok() {
                self.tx += data.len() as u64;
                self.stats(tx);
            }
        }
    }

    fn poll_flush(&mut self) {
        if self.last_flush.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_flush = Instant::now();
        if let Some(writer) = self.recorder.as_mut() {
            writer.flush();
        }
    }
}

fn worker_loop(
    mut worker: Worker,
    cmd_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<SessionEvent>,
) {
    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            if !worker.handle(cmd, &event_tx) {
                return;
            }
        }
        worker.poll_port(&event_tx);
        worker.poll_loop(&event_tx);
        worker.poll_reconnect(&event_tx);
        worker.poll_flush();
        thread::sleep(Duration::from_millis(8));
    }
}

pub struct Session {
    pub id: usize,
    pub settings: PortSettings,
    pub state: ConnectionState,
    pub auto_reconnect: bool,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub last_message: String,
    pub transferring: bool,
    pub transfer_text: String,
    pub recording_active: bool,
    pub recording_text: String,
    cmd_tx: mpsc::Sender<Command>,
    event_rx: mpsc::Receiver<SessionEvent>,
    worker: Option<JoinHandle<()>>,
}

impl Session {
    pub fn new(id: usize, settings: PortSettings, auto_reconnect: bool) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = mpsc::channel::<SessionEvent>();
        let worker_state = Worker {
            port: None,
            settings: settings.clone(),
            want_connected: false,
            auto_reconnect,
            reconnect: None,
            loop_data: Vec::new(),
            loop_interval_ms: 1000,
            loop_enabled: false,
            next_loop: Instant::now(),
            recorder: None,
            last_flush: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            rx: 0,
            tx: 0,
        };
        let worker = thread::Builder::new()
            .name(format!("serial-{}", id))
            .spawn(move || worker_loop(worker_state, cmd_rx, event_tx))
            .expect("failed to spawn serial worker");
        Self {
            id,
            settings,
            state: ConnectionState::Disconnected,
            auto_reconnect,
            rx_bytes: 0,
            tx_bytes: 0,
            last_message: "就绪".into(),
            transferring: false,
            transfer_text: String::new(),
            recording_active: false,
            recording_text: String::new(),
            cmd_tx,
            event_rx,
            worker: Some(worker),
        }
    }

    pub fn send(&self, cmd: Command) {
        let _ = self.cmd_tx.send(cmd);
    }

    pub fn drain_events(&self) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    pub fn shutdown(&mut self) {
        self.send(Command::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}
