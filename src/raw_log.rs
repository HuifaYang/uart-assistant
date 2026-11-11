use chrono::Local;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

const VOLUME_LIMIT: u64 = 10 * 1024 * 1024;

pub struct RawLogWriter {
    dir: PathBuf,
    stem: String,
    volume: u32,
    file: Option<File>,
    written: u64,
    current_path: String,
    failed: bool,
}

impl RawLogWriter {
    pub fn start(dir: PathBuf) -> Result<Self, String> {
        fs::create_dir_all(&dir).map_err(|e| format!("无法创建记录目录: {}", e))?;
        let stem = format!(
            "uart-{}-{:x}",
            Local::now().format("%Y%m%d-%H%M%S"),
            std::process::id()
        );
        let mut writer = Self {
            dir,
            stem,
            volume: 0,
            file: None,
            written: 0,
            current_path: String::new(),
            failed: false,
        };
        writer.next_volume()?;
        Ok(writer)
    }

    fn next_volume(&mut self) -> Result<(), String> {
        self.volume += 1;
        let path = self
            .dir
            .join(format!("{}-{:04}.bin", self.stem, self.volume));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| format!("无法创建记录文件: {}", e))?;
        self.file = Some(file);
        self.written = 0;
        self.current_path = path.to_string_lossy().into_owned();
        Ok(())
    }

    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        if self.failed || data.is_empty() {
            return Ok(());
        }
        let mut offset = 0usize;
        while offset < data.len() {
            if self.written >= VOLUME_LIMIT {
                self.flush();
                self.next_volume()?;
            }
            let room = (VOLUME_LIMIT - self.written) as usize;
            let len = room.min(data.len() - offset);
            let file = self.file.as_mut().ok_or("记录文件未打开")?;
            if file.write_all(&data[offset..offset + len]).is_err() {
                self.failed = true;
                return Err("写入原始记录失败".to_string());
            }
            self.written += len as u64;
            offset += len;
        }
        Ok(())
    }

    pub fn flush(&mut self) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
    }

    pub fn stop(&mut self) {
        self.flush();
        self.file.take();
    }

    pub fn failed(&self) -> bool {
        self.failed
    }

    pub fn description(&self) -> String {
        format!(
            "记录中 · 分卷 {:04} · {}",
            self.volume, self.current_path
        )
    }
}
