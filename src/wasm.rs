//! Minimal WebAssembly bindings (plain `extern "C"`, no wasm-bindgen needed).
//!
//! JS calls `init`, then each animation frame: `update(dt)` and `render()`, and
//! reads the frame straight out of wasm memory via `frame_ptr()` / `frame_len()`.
//!
//! Frame layout, 8 bytes per cell (row-major):
//!   [ascii, fg_r, fg_g, fg_b, bg_r, bg_g, bg_b, bold]

use crate::app::{sparkline, App};
use std::cell::RefCell;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static FRAME: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static SPARK: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R, default: R) -> R {
    APP.with(|a| match a.borrow_mut().as_mut() {
        Some(app) => f(app),
        None => default,
    })
}

#[no_mangle]
pub extern "C" fn init(n_x: u32, n_y: u32, seed: u32) {
    APP.with(|a| *a.borrow_mut() = Some(App::new(n_x as usize, n_y as usize, seed as u64 + 1)));
}

#[no_mangle]
pub extern "C" fn reset(n_x: u32, n_y: u32) {
    with_app(|a| a.reset(n_x as usize, n_y as usize), ());
}

#[no_mangle]
pub extern "C" fn set_param(i: u32, v: f32) {
    with_app(|a| a.set_value(i as usize, v), ());
}

#[no_mangle]
pub extern "C" fn set_paused(p: u32) {
    with_app(|a| a.paused = p != 0, ());
}

#[no_mangle]
pub extern "C" fn update(dt: f32) {
    with_app(|a| a.update(dt), ());
}

/// Render the current frame into the shared buffer. Returns the number of columns.
#[no_mangle]
pub extern "C" fn render() -> u32 {
    with_app(
        |a| {
            let g = a.frame();
            FRAME.with(|f| {
                let mut f = f.borrow_mut();
                f.clear();
                for c in &g.cells {
                    f.extend_from_slice(&[
                        c.ch as u32 as u8,
                        c.fg.0,
                        c.fg.1,
                        c.fg.2,
                        c.bg.0,
                        c.bg.1,
                        c.bg.2,
                        c.bold as u8,
                    ]);
                }
            });
            g.cols as u32
        },
        0,
    )
}

/// Number of rows in the current frame (the visible window, clamped to the map).
#[no_mangle]
pub extern "C" fn rows() -> u32 {
    with_app(|a| a.view_rows.min(a.world.h as usize) as u32, 0)
}

/// Size of the visible window in character cells (defaults to the whole map).
#[no_mangle]
pub extern "C" fn set_view(cols: u32, rows: u32) {
    with_app(|a| a.set_view(cols as usize, rows as usize), ());
}

/// Scroll the visible window by (dx, dy) character cells.
#[no_mangle]
pub extern "C" fn scroll(dx: i32, dy: i32) {
    with_app(|a| a.scroll(dx, dy), ());
}

/// Centre the camera on the map.
#[no_mangle]
pub extern "C" fn center() {
    with_app(|a| a.center(), ());
}

#[no_mangle]
pub extern "C" fn frame_ptr() -> *const u8 {
    FRAME.with(|f| f.borrow().as_ptr())
}

#[no_mangle]
pub extern "C" fn frame_len() -> u32 {
    FRAME.with(|f| f.borrow().len() as u32)
}

/// Number of lines currently in the traffic log.
#[no_mangle]
pub extern "C" fn log_len() -> u32 {
    with_app(|a| a.world.log.len() as u32, 0)
}

/// Pointer to log line `i` (0 = oldest) as UTF-8; length via `log_line_len`.
#[no_mangle]
pub extern "C" fn log_ptr(i: u32) -> *const u8 {
    with_app(
        |a| match a.world.log.get(i as usize) {
            Some(e) => SPARK.with(|b| {
                let mut b = b.borrow_mut();
                // kind byte first (0 info,1 violation,2 crash,3 event,4 tow), then text
                let k = match e.kind {
                    crate::sim::LogKind::Info => 0u8,
                    crate::sim::LogKind::Violation => 1,
                    crate::sim::LogKind::Crash => 2,
                    crate::sim::LogKind::Event => 3,
                    crate::sim::LogKind::Tow => 4,
                };
                *b = std::iter::once(k).chain(e.line().into_bytes()).collect();
                b.as_ptr()
            }),
            None => std::ptr::null(),
        },
        std::ptr::null(),
    )
}

/// Length in bytes of the buffer returned by the last `log_ptr` / `spark_ptr` call.
#[no_mangle]
pub extern "C" fn log_line_len() -> u32 {
    SPARK.with(|b| b.borrow().len() as u32)
}

/// Make the city throw a random event now.
#[no_mangle]
pub extern "C" fn trigger_event() {
    with_app(|a| a.world.trigger_event(), ());
}

/// Jump the camera to the latest incident.
#[no_mangle]
pub extern "C" fn goto_incident() {
    with_app(|a| a.goto_incident(), ());
}

/// 0 cars, 1 avg km/h, 2 stopped, 3 pedestrians, 4 flow/min, 5 exited, 6 towed, 7 red-light runs, 8 crashes
#[no_mangle]
pub extern "C" fn stat(i: u32) -> f32 {
    with_app(
        |a| {
            let s = &a.world.stats;
            match i {
                0 => s.cars as f32,
                1 => s.avg_kmh,
                2 => s.stopped as f32,
                3 => s.peds as f32,
                4 => s.flow_per_min,
                5 => s.exited as f32,
                6 => s.towed as f32,
                7 => s.red_runs as f32,
                _ => s.crashes as f32,
            }
        },
        0.0,
    )
}

/// ASCII sparkline of recent history. `which`: 0 = avg speed, 1 = jam level.
#[no_mangle]
pub extern "C" fn spark_ptr(which: u32, width: u32) -> *const u8 {
    with_app(
        |a| {
            let h = if which == 0 { &a.speed_hist } else { &a.jam_hist };
            let s = sparkline(h, width as usize);
            SPARK.with(|b| {
                let mut b = b.borrow_mut();
                *b = s.into_bytes();
                b.as_ptr()
            })
        },
        std::ptr::null(),
    )
}
