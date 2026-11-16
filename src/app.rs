//! Application state shared by the terminal UI and the web build:
//! the world, the slider values, the scrolling camera, and the little history graphs.

use crate::render::{build_base, render, Grid};
use crate::sim::*;
use std::collections::VecDeque;

pub struct SliderDef {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub step: f32,
    pub big: f32,
}

pub const SLIDERS: [SliderDef; 7] = [
    SliderDef { name: "Cars", min: 0.0, max: 600.0, step: 10.0, big: 50.0 },
    SliderDef { name: "Chaos", min: 0.0, max: 100.0, step: 5.0, big: 25.0 },
    SliderDef { name: "Pedestrians", min: 0.0, max: 300.0, step: 10.0, big: 50.0 },
    SliderDef { name: "Speed limit", min: 20.0, max: 120.0, step: 5.0, big: 20.0 },
    SliderDef { name: "Green time", min: 3.0, max: 30.0, step: 1.0, big: 5.0 },
    SliderDef { name: "Sim speed", min: 0.25, max: 4.0, step: 0.25, big: 1.0 },
    SliderDef { name: "Events", min: 0.0, max: 100.0, step: 5.0, big: 25.0 },
];

pub const HISTORY: usize = 120;

pub struct App {
    pub params: Params,
    pub sel: usize,
    pub paused: bool,
    pub world: World,
    pub base: Grid,
    pub speed_hist: VecDeque<f32>,
    pub jam_hist: VecDeque<f32>,
    /// Top-left corner of the visible window, in character cells of the full map.
    pub cam_x: usize,
    pub cam_y: usize,
    /// Size of the visible window in character cells.
    pub view_cols: usize,
    pub view_rows: usize,
    sample_acc: f32,
    seed: u64,
}

impl App {
    pub fn new(n_x: usize, n_y: usize, seed: u64) -> App {
        let params = Params::default();
        let world = World::new(n_x, n_y, seed, &params);
        let base = build_base(&world);
        // Until a front end calls `set_view`, show the whole map.
        let (view_cols, view_rows) = (base.cols, base.rows);
        App {
            params,
            sel: 0,
            paused: false,
            world,
            base,
            speed_hist: VecDeque::new(),
            jam_hist: VecDeque::new(),
            cam_x: 0,
            cam_y: 0,
            view_cols,
            view_rows,
            sample_acc: 0.0,
            seed,
        }
    }

    /// Rebuild the city (new random layout / traffic) with the same knob settings.
    pub fn reset(&mut self, n_x: usize, n_y: usize) {
        self.seed = self.seed.wrapping_add(0x9E37_79B9);
        self.world = World::new(n_x, n_y, self.seed, &self.params);
        self.base = build_base(&self.world);
        self.speed_hist.clear();
        self.jam_hist.clear();
        self.center();
    }

    // ------------------------------------------------------------- camera

    fn clamp_cam(&mut self) {
        let max_x = self.base.cols.saturating_sub(self.view_cols);
        let max_y = self.base.rows.saturating_sub(self.view_rows);
        self.cam_x = self.cam_x.min(max_x);
        self.cam_y = self.cam_y.min(max_y);
    }

    /// Size of the window (in character cells) the front end can show.
    pub fn set_view(&mut self, cols: usize, rows: usize) {
        self.view_cols = cols;
        self.view_rows = rows;
        self.clamp_cam();
    }

    /// Scroll by (dx, dy) character cells; clamped to the map.
    pub fn scroll(&mut self, dx: i32, dy: i32) {
        self.cam_x = (self.cam_x as i32 + dx).max(0) as usize;
        self.cam_y = (self.cam_y as i32 + dy).max(0) as usize;
        self.clamp_cam();
    }

    /// Put the camera in the middle of the map.
    pub fn center(&mut self) {
        self.cam_x = self.base.cols.saturating_sub(self.view_cols) / 2 & !1;
        self.cam_y = self.base.rows.saturating_sub(self.view_rows) / 2;
        self.clamp_cam();
    }

    /// Jump the camera to the most recent incident (crash, violation, event).
    pub fn goto_incident(&mut self) {
        if let Some((x, y)) = self.world.last_incident {
            self.cam_x = ((x * 2) as usize).saturating_sub(self.view_cols / 2) & !1;
            self.cam_y = (y as usize).saturating_sub(self.view_rows / 2);
            self.clamp_cam();
        }
    }

    // ------------------------------------------------------------ sliders

    pub fn value(&self, i: usize) -> f32 {
        let p = &self.params;
        match i {
            0 => p.cars as f32,
            1 => p.chaos * 100.0,
            2 => p.peds as f32,
            3 => p.speed_kmh,
            4 => p.green_secs,
            5 => p.sim_speed,
            _ => p.events * 100.0,
        }
    }

    pub fn set_value(&mut self, i: usize, v: f32) {
        let d = &SLIDERS[i];
        let v = v.clamp(d.min, d.max);
        let p = &mut self.params;
        match i {
            0 => p.cars = v.round() as u32,
            1 => p.chaos = v / 100.0,
            2 => p.peds = v.round() as u32,
            3 => p.speed_kmh = v,
            4 => p.green_secs = v,
            5 => p.sim_speed = v,
            _ => p.events = v / 100.0,
        }
    }

    pub fn fmt_value(&self, i: usize) -> String {
        let v = self.value(i);
        match i {
            1 | 6 => format!("{:.0}%", v),
            3 => format!("{:.0}km/h", v),
            4 => format!("{:.0}s", v),
            5 => format!("{:.2}x", v),
            _ => format!("{:.0}", v),
        }
    }

    pub fn frac(&self, i: usize) -> f32 {
        let d = &SLIDERS[i];
        ((self.value(i) - d.min) / (d.max - d.min)).clamp(0.0, 1.0)
    }

    pub fn select(&mut self, delta: i32) {
        let n = SLIDERS.len() as i32;
        self.sel = ((self.sel as i32 + delta).rem_euclid(n)) as usize;
    }

    pub fn adjust(&mut self, dir: i32, big: bool) {
        let d = &SLIDERS[self.sel];
        let step = if big { d.big } else { d.step };
        let v = self.value(self.sel) + step * dir as f32;
        self.set_value(self.sel, v);
    }

    // --------------------------------------------------------- simulation

    /// Advance the simulation by `dt` real seconds.
    pub fn update(&mut self, dt: f32) {
        if self.paused {
            return;
        }
        let mut sim_dt = dt.min(0.1) * self.params.sim_speed;
        let params = self.params;
        while sim_dt > 1e-6 {
            let h = sim_dt.min(0.04);
            self.world.step(h, &params);
            sim_dt -= h;
            self.sample_acc += h;
            if self.sample_acc >= 0.5 {
                self.sample_acc = 0.0;
                self.sample();
            }
        }
    }

    fn sample(&mut self) {
        let s = &self.world.stats;
        let lim = self.params.speed_kmh.max(1.0);
        let speed = (s.avg_kmh / lim).clamp(0.0, 1.0);
        let jam = if s.cars > 0 { s.stopped as f32 / s.cars as f32 } else { 0.0 };
        self.speed_hist.push_back(speed);
        self.jam_hist.push_back(jam);
        while self.speed_hist.len() > HISTORY {
            self.speed_hist.pop_front();
            self.jam_hist.pop_front();
        }
    }

    /// The visible window of the city.
    pub fn frame(&self) -> Grid {
        render(&self.world, &self.base, self.cam_x, self.cam_y, self.view_cols, self.view_rows)
    }
}

/// ASCII sparkline of the last `width` samples (values 0..1).
pub fn sparkline(hist: &VecDeque<f32>, width: usize) -> String {
    const LEVELS: &[u8] = b" .:-=+*#%@";
    let mut out = String::new();
    let n = hist.len();
    let skip = n.saturating_sub(width);
    for _ in 0..width.saturating_sub(n) {
        out.push(' ');
    }
    for v in hist.iter().skip(skip) {
        let i = (v.clamp(0.0, 1.0) * (LEVELS.len() - 1) as f32).round() as usize;
        out.push(LEVELS[i] as char);
    }
    out
}
