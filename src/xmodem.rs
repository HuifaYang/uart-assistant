use serialport::SerialPort;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

const SOH: u8 = 0x01;
const STX: u8 = 0x02;
const EOT: u8 = 0x04;
const ACK: u8 = 0x06;
const NAK: u8 = 0x15;
const CAN: u8 = 0x18;
const CRC_REQUEST: u8 = b'C';
const MAX_RETRIES: usize = 10;

fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn read_byte(port: &mut dyn SerialPort, timeout: Duration) -> Option<u8> {
    let deadline = Instant::now() + timeout;
    let mut buf = [0u8; 1];
    loop {
        match port.read(&mut buf) {
            Ok(1) => return Some(buf[0]),
            _ => {
                if Instant::now() >= deadline {
                    return None;
                }
                sleep(Duration::from_millis(5));
            }
        }
    }
}

pub fn send_file(
    port: &mut dyn SerialPort,
    data: &[u8],
    use_1k: bool,
    cancel: &Arc<AtomicBool>,
    progress: &mut impl FnMut(u64, u64),
) -> Result<(), String> {
    let mut use_crc = true;
    let mut negotiated = false;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !negotiated {
        if cancel.load(Ordering::Relaxed) {
            return Err("传输已取消".into());
        }
        if Instant::now() >= deadline {
            return Err("等待接收端就绪超时（未收到 C / NAK）".into());
        }
        match read_byte(port, Duration::from_secs(1)) {
            Some(CRC_REQUEST) => {
                use_crc = true;
                negotiated = true;
            }
            Some(NAK) => {
                use_crc = false;
                negotiated = true;
            }
            Some(CAN) => return Err("接收端取消传输".into()),
            _ => {}
        }
    }

    let block_1k = use_1k && use_crc;
    let total = data.len() as u64;
    let mut offset = 0usize;
    let mut block_no: u8 = 1;

    while offset < data.len() {
        if cancel.load(Ordering::Relaxed) {
            let _ = port.write_all(&[CAN, CAN]);
            return Err("传输已取消".into());
        }
        let size = if block_1k && data.len() - offset >= 1024 {
            1024
        } else {
            128
        };
        let mut payload = vec![0x1a; size];
        let len = size.min(data.len() - offset);
        payload[..len].copy_from_slice(&data[offset..offset + len]);

        let mut packet = Vec::with_capacity(size + 5);
        packet.push(if size == 1024 { STX } else { SOH });
        packet.push(block_no);
        packet.push(!block_no);
        packet.extend_from_slice(&payload);
        if use_crc {
            let crc = crc16(&payload);
            packet.push((crc >> 8) as u8);
            packet.push(crc as u8);
        } else {
            packet.push(payload.iter().fold(0u8, |acc, b| acc.wrapping_add(*b)));
        }

        let mut accepted = false;
        for _ in 0..MAX_RETRIES {
            if cancel.load(Ordering::Relaxed) {
                let _ = port.write_all(&[CAN, CAN]);
                return Err("传输已取消".into());
            }
            port.write_all(&packet)
                .map_err(|e| format!("发送失败: {}", e))?;
            let _ = port.flush();
            match read_byte(port, Duration::from_secs(3)) {
                Some(ACK) => {
                    accepted = true;
                    break;
                }
                Some(CAN) => return Err("接收端取消传输".into()),
                Some(NAK) | None => continue,
                _ => continue,
            }
        }
        if !accepted {
            let _ = port.write_all(&[CAN, CAN]);
            return Err(format!("第 {} 块重试 {} 次仍失败", block_no, MAX_RETRIES));
        }

        offset += len;
        block_no = block_no.wrapping_add(1);
        progress(offset as u64, total);
    }

    for _ in 0..MAX_RETRIES {
        port.write_all(&[EOT])
            .map_err(|e| format!("发送 EOT 失败: {}", e))?;
        let _ = port.flush();
        if read_byte(port, Duration::from_secs(2)) == Some(ACK) {
            progress(total, total);
            return Ok(());
        }
    }
    Err("发送 EOT 后未收到 ACK".into())
}
