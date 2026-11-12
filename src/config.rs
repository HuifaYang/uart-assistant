use crate::types::{DisplayMode, PortSettings};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MacroItem {
    pub name: String,
    pub command: String,
    pub hex: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub key: String,
    pub settings: PortSettings,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MonitorConfig {
    pub timestamps: bool,
    pub autoscroll: bool,
    pub mode: i32,
    pub encoding_index: i32,
    pub line_limit: i32,
    pub highlight: String,
    pub echo_tx: bool,
    pub loop_interval_ms: i32,
    pub append_newline: bool,
    pub hex_send: bool,
    #[serde(default = "default_log_size")]
    pub log_font_size: f32,
}

fn default_log_size() -> f32 {
    13.0
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            timestamps: false,
            autoscroll: true,
            mode: DisplayMode::Text.index(),
            encoding_index: 0,
            line_limit: 10000,
            highlight: String::new(),
            echo_tx: false,
            loop_interval_ms: 1000,
            append_newline: true,
            hex_send: false,
            log_font_size: default_log_size(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub last_settings: PortSettings,
    #[serde(default)]
    pub profiles: Vec<Profile>,
    #[serde(default)]
    pub macros: Vec<MacroItem>,
    #[serde(default)]
    pub monitor: MonitorConfig,
    #[serde(default = "default_true")]
    pub auto_reconnect: bool,
    #[serde(default)]
    pub window_width: f32,
    #[serde(default)]
    pub window_height: f32,
    #[serde(default = "default_true")]
    pub dark: bool,
    #[serde(default)]
    pub accent: i32,
    /// 界面文字颜色（#RRGGBB）；留空表示跟随主题默认值。
    #[serde(default)]
    pub text_color: String,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_settings: PortSettings::default(),
            profiles: Vec::new(),
            macros: vec![
                MacroItem { name: "Help".into(), command: "help\r\n".into(), hex: false },
                MacroItem { name: "Reboot".into(), command: "reboot\r\n".into(), hex: false },
            ],
            monitor: MonitorConfig::default(),
            auto_reconnect: true,
            window_width: 1120.0,
            window_height: 760.0,
            dark: true,
            accent: 0,
            text_color: String::new(),
        }
    }
}

pub fn config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("uart-assistant")
        .join("config.json")
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) {
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = fs::write(path, text);
        }
    }

    pub fn profile_for(&self, key: &str) -> Option<PortSettings> {
        self.profiles
            .iter()
            .find(|p| p.key == key)
            .map(|p| p.settings.clone())
    }

    pub fn remember(&mut self, key: &str, settings: &PortSettings) {
        self.last_settings = settings.clone();
        self.profiles.retain(|p| p.key != key);
        self.profiles.push(Profile {
            key: key.to_string(),
            settings: settings.clone(),
        });
        while self.profiles.len() > 40 {
            self.profiles.remove(0);
        }
    }
}
