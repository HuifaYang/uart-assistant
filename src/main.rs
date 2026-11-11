slint::include_modules!();

mod app;
mod config;
mod monitor;
mod port;
mod raw_log;
mod session;
mod terminal;
mod types;
mod xmodem;

use slint::{Timer, TimerMode};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    let dialog = MacroDialog::new()?;
    let state = Rc::new(RefCell::new(app::AppState::new(ui.as_weak())));
    app::bind(&ui, &state, dialog);
    state.borrow_mut().sync_to_ui();

    let pump = Timer::default();
    let tick_state = state.clone();
    pump.start(TimerMode::Repeated, Duration::from_millis(25), move || {
        tick_state.borrow_mut().tick();
    });

    let result = ui.run();
    state.borrow_mut().shutdown();
    result
}
