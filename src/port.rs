use crate::types::{DeviceInfo, FlowControl, Parity, PortSettings};
use serialport::{
    available_ports, DataBits, FlowControl as SpFlow, Parity as SpParity, SerialPort,
    SerialPortInfo, SerialPortType, StopBits,
};
use std::path::Path;
use std::time::Duration;

fn data_bits(v: u8) -> DataBits {
    match v {
        5 => DataBits::Five,
        6 => DataBits::Six,
        7 => DataBits::Seven,
        _ => DataBits::Eight,
    }
}

fn stop_bits(v: u8) -> StopBits {
    if v == 2 {
        StopBits::Two
    } else {
        StopBits::One
    }
}

pub fn open_port(settings: &PortSettings) -> Result<Box<dyn SerialPort>, String> {
    serialport::new(&settings.port, settings.baud)
        .timeout(Duration::from_millis(15))
        .data_bits(data_bits(settings.data_bits))
        .stop_bits(stop_bits(settings.stop_bits))
        .parity(match settings.parity {
            Parity::None => SpParity::None,
            Parity::Odd => SpParity::Odd,
            Parity::Even => SpParity::Even,
        })
        .flow_control(match settings.flow {
            FlowControl::None => SpFlow::None,
            FlowControl::RtsCts => SpFlow::Hardware,
            FlowControl::XonXoff => SpFlow::Software,
        })
        .open()
        .map_err(|e| e.to_string())
}

fn describe(info: &SerialPortInfo) -> (String, String) {
    match &info.port_type {
        SerialPortType::UsbPort(usb) => {
            let product = usb.product.clone().unwrap_or_default();
            let vendor = usb.manufacturer.clone().unwrap_or_default();
            let label = if product.is_empty() {
                format!("{} USB Serial", info.port_name)
            } else {
                format!("{} {} ({})", info.port_name, product, if vendor.is_empty() { "USB" } else { &vendor })
            };
            let detail = format!(
                "VID:{:04X} PID:{:04X}{}",
                usb.vid,
                usb.pid,
                usb.serial_number
                    .as_ref()
                    .map(|s| format!(" SN:{}", s))
                    .unwrap_or_default()
            );
            (label, detail)
        }
        other => {
            let _ = other;
            (info.port_name.clone(), "Serial port".to_string())
        }
    }
}

#[cfg(target_os = "linux")]
fn stable_aliases() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in ["/dev/serial/by-id", "/dev/serial/by-path"] {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let alias = entry.path();
            let Ok(target) = std::fs::canonicalize(&alias) else {
                continue;
            };
            let short = target.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if !short.starts_with("ttyUSB")
                && !short.starts_with("ttyACM")
                && !short.starts_with("rfcomm")
            {
                continue;
            }
            out.push((
                alias.to_string_lossy().into_owned(),
                target.to_string_lossy().into_owned(),
            ));
        }
    }
    out.sort_by(|a, b| {
        let a_id = a.0.contains("by-id");
        let b_id = b.0.contains("by-id");
        b_id.cmp(&a_id).then_with(|| a.0.cmp(&b.0))
    });
    out
}

#[cfg(not(target_os = "linux"))]
fn stable_aliases() -> Vec<(String, String)> {
    Vec::new()
}

pub fn scan_devices() -> Vec<DeviceInfo> {
    let mut devices: Vec<DeviceInfo> = Vec::new();
    let mut seen_targets: Vec<String> = Vec::new();

    for (alias, target) in stable_aliases() {
        if seen_targets.contains(&target) {
            continue;
        }
        seen_targets.push(target.clone());
        let short = Path::new(&alias)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&alias);
        devices.push(DeviceInfo {
            label: format!("{} → {}", short, Path::new(&target).file_name().unwrap_or_default().to_string_lossy()),
            detail: target.clone(),
            path: alias,
        });
    }

    if let Ok(ports) = available_ports() {
        for info in ports {
            #[cfg(target_os = "linux")]
            let canonical = std::fs::canonicalize(&info.port_name)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| info.port_name.clone());
            #[cfg(not(target_os = "linux"))]
            let canonical = info.port_name.clone();
            if seen_targets.contains(&canonical) {
                continue;
            }
            seen_targets.push(canonical);
            let (label, detail) = describe(&info);
            devices.push(DeviceInfo {
                path: info.port_name,
                label,
                detail,
            });
        }
    }
    devices
}

pub fn port_exists(path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    #[cfg(target_os = "windows")]
    {
        available_ports()
            .map(|ports| {
                ports
                    .iter()
                    .any(|p| p.port_name.eq_ignore_ascii_case(path))
            })
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Path::new(path).exists()
    }
}

pub fn set_dtr(port: &mut dyn SerialPort, level: bool) -> Result<(), String> {
    port.write_data_terminal_ready(level).map_err(|e| e.to_string())
}

pub fn set_rts(port: &mut dyn SerialPort, level: bool) -> Result<(), String> {
    port.write_request_to_send(level).map_err(|e| e.to_string())
}

pub fn stm32_sequence(port: &mut dyn SerialPort, bootloader: bool) -> Result<(), String> {
    use std::thread::sleep;
    if bootloader {
        set_rts(port, true)?;
        set_dtr(port, true)?;
        sleep(Duration::from_millis(120));
        set_dtr(port, false)?;
        sleep(Duration::from_millis(120));
        set_rts(port, false)?;
    } else {
        set_dtr(port, true)?;
        sleep(Duration::from_millis(120));
        set_dtr(port, false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn device_scan_works_off_the_ui_thread() {
        let devices = std::thread::spawn(super::scan_devices)
            .join()
            .expect("background scan panicked");
        println!("found {} serial device(s)", devices.len());
        for device in &devices {
            println!("  {} | {}", device.path, device.label);
        }
    }
}
