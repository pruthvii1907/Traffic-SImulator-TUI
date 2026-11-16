//! traffic-tui core library: simulation, ASCII renderer, app state.
//! The terminal UI lives in `main.rs` / `ui.rs`; the WebAssembly bindings in `wasm.rs`.

pub mod app;
pub mod render;
pub mod sim;

#[cfg(target_arch = "wasm32")]
pub mod wasm;
