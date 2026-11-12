#![cfg_attr(windows, windows_subsystem = "windows")]

mod config;
mod monitor;
mod port;
mod raw_log;
mod session;
mod state;
mod terminal;
mod theme;
mod types;
mod xmodem;

#[cfg(windows)]
mod dark_titlebar {
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    type Hwnd = isize;
    const ATTR: u32 = 20;
    const ATTR_OLD: u32 = 19;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(cb: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
    }

    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(hwnd: Hwnd, attr: u32, value: *const c_void, size: u32) -> i32;
    }

    static WANT_DARK: AtomicBool = AtomicBool::new(true);
    static PID: AtomicU32 = AtomicU32::new(0);

    extern "system" fn visit(hwnd: Hwnd, _lparam: isize) -> i32 {
        unsafe {
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == PID.load(Ordering::Relaxed)
                && IsWindowVisible(hwnd) != 0
                && GetWindowTextLengthW(hwnd) > 0
            {
                let value: i32 = if WANT_DARK.load(Ordering::Relaxed) { 1 } else { 0 };
                let ptr = &value as *const i32 as *const c_void;
                if DwmSetWindowAttribute(hwnd, ATTR, ptr, 4) != 0 {
                    let _ = DwmSetWindowAttribute(hwnd, ATTR_OLD, ptr, 4);
                }
            }
        }
        1
    }

    pub fn set(dark: bool) {
        WANT_DARK.store(dark, Ordering::Relaxed);
        PID.store(std::process::id(), Ordering::Relaxed);
        std::thread::spawn(|| {
            for _ in 0..12 {
                std::thread::sleep(std::time::Duration::from_millis(120));
                unsafe { EnumWindows(visit, 0) };
            }
        });
    }
}

fn set_dark_titlebar(dark: bool) {
    #[cfg(windows)]
    dark_titlebar::set(dark);
    #[cfg(not(windows))]
    let _ = dark;
}

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

fn main() -> wry::Result<()> {
    let cfg = config::Config::load();
    let dark = cfg.dark;
    let saved_size = if cfg.window_width >= 960.0 && cfg.window_height >= 600.0 {
        LogicalSize::new(cfg.window_width, cfg.window_height)
    } else {
        LogicalSize::new(1280.0, 840.0)
    };
    set_dark_titlebar(dark);
    let app = Rc::new(RefCell::new(state::App::new(cfg)));

    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("UART Assistant")
        .with_inner_size(saved_size)
        .with_min_inner_size(LogicalSize::new(960.0, 600.0))
        .build(&event_loop)
        .expect("failed to create window");

    let ipc_app = app.clone();
    // WebView2 的缓存放到用户目录，避免在程序旁边生成 *.WebView2 文件夹。
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("uart-assistant")
        .join("webview2");
    let _ = std::fs::create_dir_all(&data_dir);
    let mut web_context = wry::WebContext::new(Some(data_dir));
    let webview = match WebViewBuilder::new_with_web_context(&mut web_context)
        .with_html(include_str!("../ui/index.html"))
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            ipc_app.borrow_mut().command(req.body());
        })
        .build(&window)
    {
        Ok(webview) => webview,
        Err(err) => {
            rfd::MessageDialog::new()
                .set_title("无法启动界面")
                .set_description(format!(
                    "UART Assistant 需要系统自带的 Microsoft Edge WebView2 运行时。\n\n{}\n\n请安装 WebView2 Runtime 后重新打开。",
                    err
                ))
                .set_level(rfd::MessageLevel::Error)
                .show();
            return Ok(());
        }
    };

    app.borrow_mut().boot();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(33));
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                let size = window.inner_size().to_logical::<f32>(window.scale_factor());
                let mut state = app.borrow_mut();
                state.set_window_size(size.width, size.height);
                state.shutdown();
                *control_flow = ControlFlow::Exit;
            }
            Event::MainEventsCleared => {
                let mut state = app.borrow_mut();
                state.tick();
                if !state.out.is_empty() {
                    let payload = serde_json::Value::Array(std::mem::take(&mut state.out));
                    let script = format!("window.__host && window.__host.push({});", payload);
                    let _ = webview.evaluate_script(&script);
                }
            }
            _ => {}
        }
    });
}
