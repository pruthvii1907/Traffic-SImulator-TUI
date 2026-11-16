//! Core traffic simulation. No terminal or UI code lives here.
//!
//! Coordinate system
//! -----------------
//! The world is a grid of *tiles*. One tile is drawn as 2 characters wide and
//! 1 character tall, which makes a tile look roughly square in a terminal.
//! All distances are measured in tiles ("units"), so a car is 1 unit long.
//!
//! City layout (procedurally generated from a seed)
//! ------------------------------------------------
//! Roads are 2 tiles wide (one lane per direction, drive on the right) and run
//! the full length of the map, but they are spaced *irregularly*: every block
//! between two roads gets its own random width/height (6..~19 tiles).
//!
//! Every block has a 1-tile sidewalk ring. The interior is recursively split
//! into lots separated by 1-tile alleys; each lot becomes a building, a tower,
//! a park, or an open plaza.

use std::collections::VecDeque;

/// Smallest possible block (sidewalk ring included).
pub const MIN_BLOCK: i32 = 6;
pub const MAX_ROADS: usize = 24;
const NLANES: usize = 2 * MAX_ROADS * 2;
/// Metres per tile (only used to show km/h in the UI).
pub const UNIT_M: f32 = 2.5;
pub const CAR_COLORS: usize = 12;
pub const PED_COLORS: usize = 5;

pub fn kmh_to_units(kmh: f32) -> f32 {
    kmh / 3.6 / UNIT_M
}
pub fn units_to_kmh(u: f32) -> f32 {
    u * UNIT_M * 3.6
}

// ---------------------------------------------------------------- basic types

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Orient {
    /// Horizontal road: cars drive East (+1) or West (-1).
    H,
    /// Vertical road: cars drive South (+1) or North (-1).
    V,
}
impl Orient {
    pub fn other(self) -> Orient {
        match self {
            Orient::H => Orient::V,
            Orient::V => Orient::H,
        }
    }
    fn idx(self) -> usize {
        match self {
            Orient::H => 0,
            Orient::V => 1,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Turn {
    Straight,
    Left,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Light {
    Green,
    Yellow,
    Red,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    /// Building (style 0..4), park (style 5) or tower (style 6).
    Building(u8),
    Sidewalk,
    /// Plain road tile, tagged with the road orientation.
    Road(Orient),
    /// The 2x2 intersection box.
    Box,
    /// Zebra crossing; tagged with the orientation of the road it crosses.
    Crosswalk(Orient),
}

/// The live-tweakable knobs.
#[derive(Clone, Copy, Debug)]
pub struct Params {
    pub cars: u32,
    /// 0.0 .. 1.0
    pub chaos: f32,
    pub peds: u32,
    pub speed_kmh: f32,
    pub green_secs: f32,
    pub sim_speed: f32,
    /// 0.0 .. 1.0: how often the city throws random events at you.
    pub events: f32,
}
impl Default for Params {
    fn default() -> Self {
        Params {
            cars: 140,
            chaos: 0.25,
            peds: 80,
            speed_kmh: 50.0,
            green_secs: 8.0,
            sim_speed: 1.0,
            events: 0.5,
        }
    }
}

// ------------------------------------------------------------------------ rng

/// Tiny xorshift64* generator (keeps the crate dependency-free and wasm-friendly).
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn f32(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32) / (1u64 << 24) as f32
    }
    pub fn range(&mut self, n: usize) -> usize {
        ((self.next_u64() >> 33) as usize) % n.max(1)
    }
    pub fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
    pub fn between(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }
}

// ------------------------------------------------------------------- entities

#[derive(Clone, Debug)]
pub struct Car {
    pub orient: Orient,
    /// Index of the road this car is on (within its orientation).
    pub road: usize,
    /// +1 = East/South, -1 = West/North.
    pub dir: i32,
    /// Position of the car centre along its road axis, in tiles.
    pub pos: f32,
    pub v: f32,
    pub acc: f32,
    pub color: u8,
    /// Personal speed factor (some drivers are faster than others).
    pub v0f: f32,
    /// Personal aggressiveness 0..1 (scaled by the chaos slider).
    pub aggr: f32,
    /// Personal roll; when `roll < chaos * 0.35` this driver runs red lights.
    pub roll: f32,
    pub plan: Option<(usize, Turn)>,
    pub claim: [usize; 3],
    pub claim_n: usize,
    pub claim_entered: bool,
    pub claim_lane: u16,
    pub claim_box: (i32, i32),
    pub dawdle: f32,
    pub stuck: f32,
    /// Shown in the traffic log as "car #id".
    pub id: u32,
    /// This driver intends to jump the next red light.
    pub run_red: bool,
    /// ...and won't even look for cross traffic while doing it.
    pub blind: bool,
    /// Currently inside an intersection it entered illegally.
    pub violator: bool,
    /// Seconds of distraction left (ignores the car in front).
    pub distract: f32,
    /// Seconds left of a police chase (fast, reckless).
    pub rogue: f32,
    /// Seconds left of a breakdown (stopped, hazard lights).
    pub broken: f32,
    /// Crashed: stuck in place until the tow truck arrives.
    pub wrecked: bool,
    pub wreck: f32,
}

#[derive(Clone, Debug)]
pub struct Ped {
    pub tx: i32,
    pub ty: i32,
    pub nx: i32,
    pub ny: i32,
    pub t: f32,
    pub moving: bool,
    pub wait: f32,
    pub speed: f32,
    pub dx: i32,
    pub dy: i32,
    pub want: Option<(i32, i32)>,
    pub color: u8,
}

#[derive(Clone, Debug)]
pub struct IBox {
    pub x0: i32,
    pub y0: i32,
    pub offset: f64,
    /// While `time < dark_until` the signal is out and flashes yellow.
    pub dark_until: f64,
}

#[derive(Clone, Debug)]
pub struct Crosswalk {
    pub orient: Orient,
    pub road: usize,
    pub k: i32,
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub cars: usize,
    pub avg_kmh: f32,
    pub stopped: usize,
    pub peds: usize,
    pub flow_per_min: f32,
    pub exited: u64,
    pub towed: u64,
    pub red_runs: u64,
    pub crashes: u64,
}

/// Closing speed (tiles/s) above which hitting the car in front is a crash.
const REAR_END_SPEED: f32 = 2.6;
const LOG_CAP: usize = 400;

/// What kind of line this is in the traffic log.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogKind {
    Info,
    Violation,
    Crash,
    Event,
    Tow,
}

#[derive(Clone, Debug)]
pub struct LogEntry {
    /// Simulation time in seconds.
    pub t: f64,
    pub kind: LogKind,
    pub text: String,
    /// Tile position of the incident, or (-1, -1) when it has none.
    pub x: i32,
    pub y: i32,
}
impl LogEntry {
    /// "[mm:ss] text"
    pub fn line(&self) -> String {
        let s = self.t as u64;
        format!("[{:02}:{:02}] {}", s / 60, s % 60, self.text)
    }
}

/// City-wide events that run for a while.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EventKind {
    RushHour,
    Rain,
    CrowdSurge,
    PoliceChase,
    Outage,
}
impl EventKind {
    pub fn label(self) -> &'static str {
        match self {
            EventKind::RushHour => "RUSH HOUR",
            EventKind::Rain => "RAIN",
            EventKind::CrowdSurge => "CROWD SURGE",
            EventKind::PoliceChase => "POLICE CHASE",
            EventKind::Outage => "POWER OUTAGE",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ActiveEvent {
    pub kind: EventKind,
    /// Simulation time when it ends.
    pub until: f64,
}

/// An oil slick on one road tile.
#[derive(Clone, Debug)]
pub struct Hazard {
    pub x: i32,
    pub y: i32,
    pub until: f64,
    pub hits: u8,
}

fn wreck_car(c: &mut Car, dur: f32) {
    c.wreck = c.wreck.max(dur);
    c.wrecked = true;
    c.v = 0.0;
    c.acc = 0.0;
    c.rogue = 0.0;
    c.run_red = false;
    c.blind = false;
    c.violator = false;
    c.distract = 0.0;
    c.broken = 0.0;
}

pub struct World {
    /// Number of vertical roads / horizontal roads.
    pub n_x: usize,
    pub n_y: usize,
    pub w: i32,
    pub h: i32,
    /// X of the first tile of each vertical road.
    pub vx: Vec<i32>,
    /// Y of the first tile of each horizontal road.
    pub hy: Vec<i32>,
    pub tiles: Vec<Tile>,
    pub crosswalks: Vec<Crosswalk>,
    cw_at: Vec<i32>,
    sidewalk: Vec<usize>,
    pub boxes: Vec<IBox>,
    pub cars: Vec<Car>,
    pub peds: Vec<Ped>,
    pub time: f64,
    pub stats: Stats,
    rng: Rng,
    green: f32,
    // scratch buffers, rebuilt every step
    lanes: Vec<Vec<usize>>,
    leader: Vec<Option<usize>>,
    claimed: Vec<u16>,
    ped_occ: Vec<bool>,
    spawn_acc: f32,
    ped_acc: f32,
    exits: VecDeque<f64>,
    exited_total: u64,
    towed_total: u64,
    next_id: u32,
    /// The traffic log, oldest first.
    pub log: VecDeque<LogEntry>,
    /// City-wide events that are running right now.
    pub active: Vec<ActiveEvent>,
    pub hazards: Vec<Hazard>,
    /// Where the latest crash / violation / event happened (tile coordinates).
    pub last_incident: Option<(i32, i32)>,
    hotspot: Option<(i32, i32)>,
    next_event: f64,
    near_miss_cd: f64,
    event_now: bool,
    jam_state: bool,
    red_runs: u64,
    crashes: u64,
}

// ------------------------------------------------------------------- helpers

fn lane_id(orient: Orient, road: usize, dir: i32) -> usize {
    (orient.idx() * MAX_ROADS + road) * 2 + (dir > 0) as usize
}

fn tile_xy(orient: Orient, along: i32, cross: i32) -> (i32, i32) {
    match orient {
        Orient::H => (along, cross),
        Orient::V => (cross, along),
    }
}

/// Direction after turning.
fn turn_dir(orient: Orient, dir: i32, turn: Turn) -> i32 {
    match (orient, turn) {
        (_, Turn::Straight) => dir,
        (Orient::H, Turn::Right) => dir,
        (Orient::H, Turn::Left) => -dir,
        (Orient::V, Turn::Right) => -dir,
        (Orient::V, Turn::Left) => dir,
    }
}

/// Intelligent Driver Model acceleration.
#[allow(clippy::too_many_arguments)]
fn idm(v: f32, v0: f32, gap: f32, vl: f32, a: f32, b: f32, s0: f32, t: f32) -> f32 {
    let gap = gap.max(0.05);
    let dv = v - vl;
    let sstar = s0 + (v * t + v * dv / (2.0 * (a * b).sqrt())).max(0.0);
    a * (1.0 - (v / v0.max(0.5)).powi(4) - (sstar / gap) * (sstar / gap))
}

// ------------------------------------------------------- procedural generation

/// Random road layout along one axis: where each of the `n` roads starts, and
/// the size of each of the `n + 1` blocks around them.
fn layout(n: usize, rng: &mut Rng) -> (Vec<i32>, Vec<i32>) {
    let mut starts = Vec::with_capacity(n);
    let mut blocks = Vec::with_capacity(n + 1);
    let mut pos = 0;
    for i in 0..=n {
        let mut b = MIN_BLOCK + rng.range(8) as i32;
        // Every now and then a really big block (campus, park, plaza ...).
        if rng.chance(0.3) {
            b += rng.range(7) as i32;
        }
        blocks.push(b);
        pos += b;
        if i < n {
            starts.push(pos);
            pos += 2;
        }
    }
    (starts, blocks)
}

/// Recursively split a block interior into lots separated by 1-tile alleys
/// (left as sidewalk) and fill each lot with a building, tower, park or plaza.
#[allow(clippy::too_many_arguments)]
fn gen_lots(tiles: &mut [Tile], w: i32, x: i32, y: i32, bw: i32, bh: i32, depth: u32, rng: &mut Rng) {
    if bw <= 0 || bh <= 0 {
        return;
    }
    let can_x = bw >= 8;
    let can_y = bh >= 8;
    if depth < 3 && (can_x || can_y) && rng.chance(0.85) {
        if can_x && (!can_y || bw >= bh) {
            let s = 3 + rng.range((bw - 6) as usize) as i32;
            gen_lots(tiles, w, x, y, s, bh, depth + 1, rng);
            gen_lots(tiles, w, x + s + 1, y, bw - s - 1, bh, depth + 1, rng);
        } else {
            let s = 3 + rng.range((bh - 6) as usize) as i32;
            gen_lots(tiles, w, x, y, bw, s, depth + 1, rng);
            gen_lots(tiles, w, x, y + s + 1, bw, bh - s - 1, depth + 1, rng);
        }
        return;
    }
    // Leaf lot.
    let r = rng.range(100);
    let style: u8 = if r < 8 {
        return; // open plaza: stays sidewalk
    } else if r < 24 {
        5 // park
    } else if r < 36 {
        6 // tower
    } else {
        rng.range(5) as u8
    };
    for yy in 0..bh {
        for xx in 0..bw {
            tiles[((y + yy) * w + (x + xx)) as usize] = Tile::Building(style);
        }
    }
}

// ---------------------------------------------------------------------- world

impl World {
    pub fn new(n_x: usize, n_y: usize, seed: u64, p: &Params) -> World {
        let n_x = n_x.clamp(1, MAX_ROADS);
        let n_y = n_y.clamp(1, MAX_ROADS);
        let mut rng = Rng::new(seed);
        let (vx, bw) = layout(n_x, &mut rng);
        let (hy, bh) = layout(n_y, &mut rng);
        let w = vx[n_x - 1] + 2 + bw[n_x];
        let h = hy[n_y - 1] + 2 + bh[n_y];
        let idx = |x: i32, y: i32| (y * w + x) as usize;
        let mut tiles = vec![Tile::Sidewalk; (w * h) as usize];

        // City blocks: sidewalk ring + procedurally subdivided interior.
        for by in 0..=n_y {
            let y0 = if by == 0 { 0 } else { hy[by - 1] + 2 };
            for bx in 0..=n_x {
                let x0 = if bx == 0 { 0 } else { vx[bx - 1] + 2 };
                gen_lots(&mut tiles, w, x0 + 1, y0 + 1, bw[bx] - 2, bh[by] - 2, 0, &mut rng);
            }
        }
        // Roads.
        for i in 0..n_x {
            let xs = vx[i];
            for y in 0..h {
                for dx in 0..2 {
                    tiles[idx(xs + dx, y)] = Tile::Road(Orient::V);
                }
            }
        }
        for j in 0..n_y {
            let ys = hy[j];
            for x in 0..w {
                for dy in 0..2 {
                    tiles[idx(x, ys + dy)] = Tile::Road(Orient::H);
                }
            }
        }
        // Intersections + zebra crossings on all four sides of each.
        let mut boxes = Vec::new();
        let mut crosswalks: Vec<Crosswalk> = Vec::new();
        let mut cw_at = vec![-1i32; (w * h) as usize];
        for j in 0..n_y {
            for i in 0..n_x {
                let (x0, y0) = (vx[i], hy[j]);
                for &xk in &[x0 - 1, x0 + 2] {
                    let id = crosswalks.len() as i32;
                    crosswalks.push(Crosswalk { orient: Orient::H, road: j, k: xk });
                    for dy in 0..2 {
                        let t = idx(xk, y0 + dy);
                        tiles[t] = Tile::Crosswalk(Orient::H);
                        cw_at[t] = id;
                    }
                }
                for &yk in &[y0 - 1, y0 + 2] {
                    let id = crosswalks.len() as i32;
                    crosswalks.push(Crosswalk { orient: Orient::V, road: i, k: yk });
                    for dx in 0..2 {
                        let t = idx(x0 + dx, yk);
                        tiles[t] = Tile::Crosswalk(Orient::V);
                        cw_at[t] = id;
                    }
                }
                for dy in 0..2 {
                    for dx in 0..2 {
                        tiles[idx(x0 + dx, y0 + dy)] = Tile::Box;
                    }
                }
                // Lights are staggered diagonally so platoons catch a "green wave",
                // plus jitter so the uneven city does not look robotic.
                let wave = (i + j) as f64 * 1.45;
                boxes.push(IBox { x0, y0, offset: wave + rng.f32() as f64 * 2.0, dark_until: 0.0 });
            }
        }
        let sidewalk: Vec<usize> = tiles
            .iter()
            .enumerate()
            .filter(|(_, t)| **t == Tile::Sidewalk)
            .map(|(i, _)| i)
            .collect();

        let mut world = World {
            n_x,
            n_y,
            w,
            h,
            vx,
            hy,
            tiles,
            crosswalks,
            cw_at,
            sidewalk,
            boxes,
            cars: Vec::new(),
            peds: Vec::new(),
            time: 0.0,
            stats: Stats::default(),
            rng,
            green: p.green_secs,
            lanes: vec![Vec::new(); NLANES],
            leader: Vec::new(),
            claimed: vec![0; (w * h) as usize],
            ped_occ: vec![false; (w * h) as usize],
            spawn_acc: 0.0,
            ped_acc: 0.0,
            exits: VecDeque::new(),
            exited_total: 0,
            towed_total: 0,
            next_id: 1,
            log: VecDeque::new(),
            active: Vec::new(),
            hazards: Vec::new(),
            last_incident: None,
            hotspot: None,
            next_event: 8.0,
            near_miss_cd: 0.0,
            event_now: false,
            jam_state: false,
            red_runs: 0,
            crashes: 0,
        };
        // Pre-populate so the city is already alive on the first frame.
        let mut tries = 0;
        while world.cars.len() < p.cars as usize && tries < p.cars * 30 + 50 {
            world.spawn_car(p, true);
            tries += 1;
        }
        for _ in 0..p.peds {
            world.spawn_ped();
        }
        world.update_stats();
        world
    }

    // ------------------------------------------------------------ geometry

    /// First tile of road `idx` among the roads of orientation `o`
    /// (`H` roads are positioned by their y, `V` roads by their x).
    pub fn road_pos(&self, o: Orient, idx: usize) -> i32 {
        match o {
            Orient::H => self.hy[idx],
            Orient::V => self.vx[idx],
        }
    }

    /// True for the second (lower / right) tile row or column of a road.
    pub fn is_second_lane(&self, o: Orient, x: i32, y: i32) -> bool {
        match o {
            Orient::H => self.hy.iter().any(|&s| s + 1 == y),
            Orient::V => self.vx.iter().any(|&s| s + 1 == x),
        }
    }

    /// The row/column a lane occupies (drive on the right).
    pub fn lane_cross(&self, orient: Orient, road: usize, dir: i32) -> i32 {
        let s = self.road_pos(orient, road);
        match orient {
            // East uses the lower row, West the upper row.
            Orient::H => s + if dir > 0 { 1 } else { 0 },
            // South uses the left column, North the right column.
            Orient::V => s + if dir > 0 { 0 } else { 1 },
        }
    }

    fn extent(&self, o: Orient) -> f32 {
        match o {
            Orient::H => self.w as f32,
            Orient::V => self.h as f32,
        }
    }

    fn crossing_count(&self, o: Orient) -> usize {
        match o {
            Orient::H => self.n_x,
            Orient::V => self.n_y,
        }
    }

    /// Index into `boxes` for a car on (orient, road) crossing road `k`.
    fn box_index(&self, o: Orient, road: usize, k: usize) -> usize {
        match o {
            Orient::H => road * self.n_x + k,
            Orient::V => k * self.n_x + road,
        }
    }

    fn box_origin(&self, o: Orient, road: usize, k: usize) -> (i32, i32) {
        match o {
            Orient::H => (self.vx[k], self.hy[road]),
            Orient::V => (self.vx[road], self.hy[k]),
        }
    }

    /// Centre of a car in tile coordinates.
    pub fn car_center(&self, c: &Car) -> (f32, f32) {
        let cross = self.lane_cross(c.orient, c.road, c.dir) as f32 + 0.5;
        match c.orient {
            Orient::H => (c.pos, cross),
            Orient::V => (cross, c.pos),
        }
    }

    pub fn tile(&self, x: i32, y: i32) -> Tile {
        self.tiles[(y * self.w + x) as usize]
    }

    fn walkable(&self, x: i32, y: i32) -> bool {
        x >= 0
            && y >= 0
            && x < self.w
            && y < self.h
            && matches!(self.tile(x, y), Tile::Sidewalk | Tile::Crosswalk(_))
    }

    // -------------------------------------------------------------- lights

    pub fn light(&self, b: usize, o: Orient) -> Light {
        self.light_info(b, o).0
    }

    /// Signal state plus how long (seconds) it has been in the yellow/red state.
    pub fn light_info(&self, b: usize, o: Orient) -> (Light, f32) {
        // During a power outage the signal flashes yellow, which turns the
        // junction into a cautious give-way crossing.
        if self.time < self.boxes[b].dark_until {
            return (Light::Yellow, 0.0);
        }
        let g = self.green as f64;
        let (y, r) = (1.6, 0.7);
        let half = g + y + r;
        let total = 2.0 * half;
        let t = (self.time + self.boxes[b].offset).rem_euclid(total);
        // North/South traffic is green first, East/West half a cycle later.
        let start = if o == Orient::V { 0.0 } else { half };
        let rel = (t - start).rem_euclid(total);
        if rel < g {
            (Light::Green, 0.0)
        } else if rel < g + y {
            (Light::Yellow, (rel - g) as f32)
        } else {
            (Light::Red, (rel - g - y) as f32)
        }
    }

    // --------------------------------------------------------------- spawn

    fn spawn_car(&mut self, p: &Params, random_pos: bool) -> bool {
        let total = (self.n_x + self.n_y) * 2;
        let r = self.rng.range(total);
        let dir = if r % 2 == 0 { 1 } else { -1 };
        let k = r / 2;
        let (orient, road) = if k < self.n_y { (Orient::H, k) } else { (Orient::V, k - self.n_y) };
        let ext = self.extent(orient);
        let pos = if random_pos {
            self.rng.between(0.5, ext - 0.5)
        } else if dir > 0 {
            -0.5
        } else {
            ext + 0.5
        };
        let min_gap = if random_pos { 2.3 } else { 3.6 };
        if self
            .cars
            .iter()
            .any(|c| c.orient == orient && c.road == road && c.dir == dir && (c.pos - pos).abs() < min_gap)
        {
            return false;
        }
        if random_pos {
            for m in 0..self.crossing_count(orient) {
                let cs = self.road_pos(orient.other(), m) as f32;
                if pos > cs - 2.8 && pos < cs + 4.8 {
                    return false;
                }
            }
        }
        let lim = kmh_to_units(p.speed_kmh);
        let car = Car {
            orient,
            road,
            dir,
            pos,
            v: lim * 0.6,
            acc: 0.0,
            color: self.rng.range(CAR_COLORS) as u8,
            v0f: self.rng.between(0.85, 1.1),
            aggr: self.rng.f32(),
            roll: self.rng.f32(),
            plan: None,
            claim: [0; 3],
            claim_n: 0,
            claim_entered: false,
            claim_lane: 0,
            claim_box: (0, 0),
            dawdle: 0.0,
            stuck: 0.0,
            id: self.next_id,
            run_red: false,
            blind: false,
            violator: false,
            distract: 0.0,
            rogue: 0.0,
            broken: 0.0,
            wrecked: false,
            wreck: 0.0,
        };
        self.next_id += 1;
        self.cars.push(car);
        true
    }

    fn spawn_ped(&mut self) {
        if self.sidewalk.is_empty() {
            return;
        }
        let mut t = self.sidewalk[self.rng.range(self.sidewalk.len())];
        // During a crowd surge most newcomers appear around the hotspot.
        if let Some((hx, hy)) = self.hotspot {
            for _ in 0..12 {
                let x = hx + self.rng.range(25) as i32 - 12;
                let y = hy + self.rng.range(25) as i32 - 12;
                if x >= 0 && y >= 0 && x < self.w && y < self.h && self.tile(x, y) == Tile::Sidewalk {
                    t = (y * self.w + x) as usize;
                    break;
                }
            }
        }
        let (tx, ty) = ((t as i32) % self.w, (t as i32) / self.w);
        let ped = Ped {
            tx,
            ty,
            nx: tx,
            ny: ty,
            t: 0.0,
            moving: false,
            wait: self.rng.between(0.0, 1.0),
            speed: self.rng.between(0.8, 1.35),
            dx: 0,
            dy: 0,
            want: None,
            color: self.rng.range(PED_COLORS) as u8,
        };
        self.peds.push(ped);
    }

    // ---------------------------------------------------------------- step

    pub fn step(&mut self, dt: f32, p: &Params) {
        self.time += dt as f64;
        self.green = p.green_secs;
        self.run_events(p);
        self.rebuild_scratch();
        for i in 0..self.cars.len() {
            self.update_car(i, dt, p);
        }
        self.update_hazards();
        self.detect_collisions();
        self.cull();
        self.spawn_cars(dt, p);
        self.update_peds(dt, p);
        self.update_stats();
    }

    fn rebuild_scratch(&mut self) {
        let mut lanes = std::mem::take(&mut self.lanes);
        for l in lanes.iter_mut() {
            l.clear();
        }
        for (i, c) in self.cars.iter().enumerate() {
            lanes[lane_id(c.orient, c.road, c.dir)].push(i);
        }
        self.leader.clear();
        self.leader.resize(self.cars.len(), None);
        for l in lanes.iter_mut() {
            l.sort_by(|&a, &b| {
                let pa = self.cars[a].pos * self.cars[a].dir as f32;
                let pb = self.cars[b].pos * self.cars[b].dir as f32;
                pa.partial_cmp(&pb).unwrap_or(std::cmp::Ordering::Equal)
            });
            for w in l.windows(2) {
                self.leader[w[0]] = Some(w[1]);
            }
        }
        self.lanes = lanes;

        self.claimed.iter_mut().for_each(|c| *c = 0);
        for c in &self.cars {
            for t in &c.claim[..c.claim_n] {
                self.claimed[*t] = c.claim_lane;
            }
        }
        self.ped_occ.iter_mut().for_each(|c| *c = false);
        for pd in &self.peds {
            let cur = (pd.ty * self.w + pd.tx) as usize;
            let nxt = (pd.ny * self.w + pd.nx) as usize;
            if self.cw_at[cur] >= 0 {
                self.ped_occ[cur] = true;
            }
            if pd.moving && self.cw_at[nxt] >= 0 {
                self.ped_occ[nxt] = true;
            }
        }
    }

    /// Next intersection whose box the car has not yet fully left.
    fn next_box(&self, c: &Car) -> Option<usize> {
        let n = self.crossing_count(c.orient);
        let dir = c.dir as f32;
        for k in 0..n {
            let order = if c.dir > 0 { k } else { n - 1 - k };
            let cs = self.road_pos(c.orient.other(), order) as f32;
            let exit_edge = if c.dir > 0 { cs + 2.0 } else { cs };
            if (exit_edge - c.pos) * dir > 0.0 {
                return Some(order);
            }
        }
        None
    }

    fn turn_center(&self, c: &Car, k: usize, turn: Turn) -> f32 {
        let nd = turn_dir(c.orient, c.dir, turn);
        self.lane_cross(c.orient.other(), k, nd) as f32 + 0.5
    }

    fn pick_turn(&mut self) -> Turn {
        let r = self.rng.f32();
        if r < 0.62 {
            Turn::Straight
        } else if r < 0.81 {
            Turn::Left
        } else {
            Turn::Right
        }
    }

    /// Tiles inside the intersection box that this car will sweep through.
    fn path_tiles(&self, c: &Car, k: usize, turn: Turn) -> ([usize; 3], usize) {
        let (bx, by) = self.box_origin(c.orient, c.road, k);
        let (a0, a1) = match c.orient {
            Orient::H => (bx, bx + 1),
            Orient::V => (by, by + 1),
        };
        let cross0 = self.lane_cross(c.orient, c.road, c.dir);
        let start = if c.dir > 0 { a0 } else { a1 };
        let nd = turn_dir(c.orient, c.dir, turn);
        let end = match turn {
            Turn::Straight => {
                if c.dir > 0 {
                    a1
                } else {
                    a0
                }
            }
            _ => self.lane_cross(c.orient.other(), k, nd),
        };
        let mut out = [0usize; 3];
        let mut n = 0;
        let mut a = start;
        loop {
            let (x, y) = tile_xy(c.orient, a, cross0);
            out[n] = (y * self.w + x) as usize;
            n += 1;
            if a == end || n >= 3 {
                break;
            }
            a += c.dir;
        }
        if turn != Turn::Straight && n < 3 {
            let no = c.orient.other();
            let (b0, b1) = match no {
                Orient::H => (bx, bx + 1),
                Orient::V => (by, by + 1),
            };
            let na = cross0 + nd;
            if na >= b0 && na <= b1 {
                let (x, y) = tile_xy(no, na, end);
                out[n] = (y * self.w + x) as usize;
                n += 1;
            }
        }
        (out, n)
    }

    /// Is there room for this car on the road it is about to drive onto?
    fn exit_clear(&self, c: &Car, k: usize, turn: Turn, leader: Option<usize>) -> bool {
        if turn == Turn::Straight {
            if let Some(l) = leader {
                let o = &self.cars[l];
                let cs = self.road_pos(c.orient.other(), k) as f32;
                let exit_edge = if c.dir > 0 { cs + 2.0 } else { cs };
                let rel = (o.pos - exit_edge) * c.dir as f32;
                if o.v < 1.5 && rel < 2.2 {
                    return false;
                }
            }
            true
        } else {
            let nd = turn_dir(c.orient, c.dir, turn);
            let lid = lane_id(c.orient.other(), k, nd);
            let e = self.lane_cross(c.orient, c.road, c.dir) as f32 + 0.5;
            for &d in &self.lanes[lid] {
                let o = &self.cars[d];
                let rel = (o.pos - e) * nd as f32;
                if rel > -(1.8 + 0.5 * o.v) && rel < 1.4 {
                    return false;
                }
                if o.v < 1.5 && rel >= -0.4 && rel < 3.2 {
                    return false;
                }
            }
            true
        }
    }

    /// Reserve the intersection tiles for this car (or fail if somebody with a
    /// conflicting path already holds them).
    fn try_claim(&mut self, c: &mut Car, k: usize, turn: Turn, force: bool, leader: Option<usize>) -> bool {
        let (tiles, n) = self.path_tiles(c, k, turn);
        // Only cars on an identical path (same lane, same turn) may share tiles.
        let turn_idx = match turn {
            Turn::Straight => 0,
            Turn::Left => 1,
            Turn::Right => 2,
        };
        let code = (lane_id(c.orient, c.road, c.dir) * 3 + turn_idx) as u16 + 1;
        if !force {
            for t in &tiles[..n] {
                let o = self.claimed[*t];
                if o != 0 && o != code {
                    return false;
                }
            }
            if !self.exit_clear(c, k, turn, leader) {
                return false;
            }
        }
        for t in &tiles[..n] {
            self.claimed[*t] = code;
        }
        c.claim = tiles;
        c.claim_n = n;
        c.claim_entered = false;
        c.claim_lane = code;
        c.claim_box = self.box_origin(c.orient, c.road, k);
        true
    }

    fn cw_occupied(&self, o: Orient, road: usize, k: i32) -> bool {
        let r = self.road_pos(o, road);
        let (a, b) = match o {
            Orient::H => (k + r * self.w, k + (r + 1) * self.w),
            Orient::V => (r + k * self.w, r + 1 + k * self.w),
        };
        self.ped_occ[a as usize] || self.ped_occ[b as usize]
    }

    fn update_car(&mut self, i: usize, dt: f32, p: &Params) {
        let mut c = self.cars[i].clone();
        // A wreck just sits there until the tow truck comes (see `cull`).
        if c.wrecked {
            c.wreck -= dt;
            c.v = 0.0;
            c.acc = 0.0;
            self.cars[i] = c;
            return;
        }
        let dir = c.dir as f32;
        let rain = self.is_active(EventKind::Rain);
        let chaos = (p.chaos + if rain { 0.12 } else { 0.0 }).min(1.0);
        let rogue = c.rogue > 0.0;
        if rogue {
            c.rogue -= dt;
        }
        let broken = c.broken > 0.0;
        if broken {
            c.broken -= dt;
            if c.broken <= 0.0 {
                let id = c.id;
                self.note(LogKind::Info, format!("Car #{} restarted and is moving again", id), -1, -1);
            }
        }
        let distracted = c.distract > 0.0;
        let mut v0 = kmh_to_units(p.speed_kmh * if rain { 0.82 } else { 1.0 }) * c.v0f;
        if rogue {
            v0 *= 1.7;
        }
        let a_max = 3.0 * (1.0 + 0.5 * chaos * c.aggr) * if rogue { 1.4 } else { 1.0 };
        let b = 3.5;
        let t_head = (1.35 - chaos * c.aggr).max(0.3);
        let s0 = 0.45;
        let reckless = rogue || c.roll < chaos * 0.35;
        let front = c.pos + 0.5 * dir;
        let leader = self.leader[i];

        let nbox = self.next_box(&c);
        if let Some(k) = nbox {
            if c.plan.map(|pl| pl.0) != Some(k) {
                let t = self.pick_turn();
                c.plan = Some((k, t));
                // Decide once per junction whether this driver will jump a red.
                c.run_red = rogue || (c.roll < chaos * 0.35 && self.rng.chance(0.18));
                c.blind = c.run_red && self.rng.chance(if rogue { 0.5 } else { 0.15 + 0.4 * chaos });
            }
        }

        // Slow down for turns.
        let mut dist_turn = f32::INFINITY;
        if let (Some(k), Some((pk, turn))) = (nbox, c.plan) {
            if pk == k && turn != Turn::Straight {
                let tc = self.turn_center(&c, k, turn);
                let d = (tc - c.pos) * dir;
                if d > -0.01 {
                    dist_turn = d;
                    v0 = v0.min(3.0 + 1.3 * d);
                }
            }
        }

        let mut acc = a_max * (1.0 - (c.v / v0.max(0.5)).powi(4));

        // 1) car in front on the same lane (a distracted driver doesn't see it)
        if let (Some(l), false) = (leader, distracted) {
            let o = &self.cars[l];
            let gap = (o.pos - c.pos) * dir - 1.0;
            acc = acc.min(idm(c.v, v0, gap, o.v, a_max, b, s0, t_head));
        }

        // 2) car in front on the lane we are about to turn onto
        if dist_turn.is_finite() {
            if let Some((k, turn)) = c.plan {
                let nd = turn_dir(c.orient, c.dir, turn);
                let lid = lane_id(c.orient.other(), k, nd);
                let e = self.lane_cross(c.orient, c.road, c.dir) as f32 + 0.5;
                let mut best: Option<(f32, f32)> = None;
                for &d in &self.lanes[lid] {
                    let o = &self.cars[d];
                    let rel = (o.pos - e) * nd as f32;
                    if rel >= -0.4 && best.map_or(true, |(r, _)| rel < r) {
                        best = Some((rel, o.v));
                    }
                }
                if let Some((rel, vl)) = best {
                    let gap = dist_turn + rel - 1.0;
                    acc = acc.min(idm(c.v, v0, gap, vl, a_max, b, s0, t_head));
                }
            }
        }

        // 3) traffic light / intersection reservation
        if let Some(k) = nbox {
            let cs = self.road_pos(c.orient.other(), k) as f32;
            let entry = if c.dir > 0 { cs } else { cs + 2.0 };
            let in_box = (entry - c.pos) * dir <= 0.0;
            if !in_box && c.claim_n == 0 {
                let turn = c.plan.map(|pl| pl.1).unwrap_or(Turn::Straight);
                let stop_line = entry - dir;
                let gap_stop = (stop_line - front) * dir;
                let stopdist = c.v * c.v / (2.0 * b);
                let (light, elapsed) = self.light_info(self.box_index(c.orient, c.road, k), c.orient);
                // A left-turner already waiting at the line may "sneak" through at the end of the phase.
                let sneak = turn == Turn::Left && c.v < 1.5 && gap_stop < 1.5 && (light == Light::Yellow || elapsed < 1.3);
                let permitted = match light {
                    Light::Green => true,
                    Light::Yellow => reckless || sneak || stopdist > gap_stop - 0.2,
                    // jumpers go through just after the light turns red, not after waiting ages
                    Light::Red => (c.run_red && (elapsed < 3.0 || rogue)) || sneak,
                } || gap_stop < -0.3;
                // Never reserve the box ahead of the car in front of us (that would deadlock).
                let leader_ok = match leader {
                    None => true,
                    Some(l) => {
                        let o = &self.cars[l];
                        (o.pos - entry) * dir > 0.0
                            || (o.claim_n > 0 && o.claim_box == self.box_origin(c.orient, c.road, k))
                    }
                };
                let mut ok = false;
                if permitted && leader_ok && (gap_stop < stopdist + 2.5) {
                    let jumping = light == Light::Red && c.run_red && !sneak && gap_stop >= -0.3 && elapsed > 0.4;
                    if jumping && c.blind {
                        // Doesn't check for cross traffic: takes the junction whatever is in it.
                        self.try_claim(&mut c, k, turn, true, leader);
                        c.violator = true;
                        ok = true;
                    } else {
                        ok = self.try_claim(&mut c, k, turn, false, leader);
                    }
                    if ok && jumping {
                        let (bx, by) = self.box_origin(c.orient, c.road, k);
                        let place = self.place(bx, by);
                        let text = format!(
                            "Car #{} ran a red light at {} (red for {:.1}s{})",
                            c.id,
                            place,
                            elapsed,
                            if c.blind { ", didn't look" } else { "" }
                        );
                        self.red_runs += 1;
                        self.note(LogKind::Violation, text, bx, by);
                    }
                }
                if !ok {
                    if gap_stop >= -0.3 {
                        acc = acc.min(idm(c.v, v0, gap_stop, 0.0, a_max, b, s0, t_head));
                    } else {
                        // Overshot the stop line without a reservation: brake for the box itself.
                        let gap_entry = (entry - front) * dir;
                        if gap_entry > 0.1 {
                            acc = acc.min(idm(c.v, v0, gap_entry, 0.0, a_max, b, s0, t_head));
                        } else {
                            self.try_claim(&mut c, k, turn, true, leader);
                        }
                    }
                }
            }
        }

        // 4) pedestrians on a zebra crossing ahead
        for m in 0..self.crossing_count(c.orient) {
            let cs = self.road_pos(c.orient.other(), m);
            for kk in [cs - 1, cs + 2] {
                let near = if c.dir > 0 { kk as f32 } else { kk as f32 + 1.0 };
                let dist = (near - front) * dir;
                if dist > -0.2 && dist < 14.0 && self.cw_occupied(c.orient, c.road, kk) {
                    // Only react if we can still stop (nobody gets hurt here).
                    if dist > c.v * c.v / 18.0 - 0.2 {
                        acc = acc.min(idm(c.v, v0, dist, 0.0, a_max, b, s0, t_head));
                    }
                }
            }
        }

        // 5) chaos: random brake blips and jittery throttle
        if c.dawdle > 0.0 {
            c.dawdle -= dt;
            acc = acc.min(-2.5);
        } else {
            let pb = (chaos * chaos * 0.5 + chaos * 0.1) * dt * c.aggr.max(0.3);
            if self.rng.chance(pb) {
                c.dawdle = self.rng.between(0.3, 0.9);
            }
        }
        // ...and sometimes a driver looks at their phone.
        if c.distract > 0.0 {
            c.distract -= dt;
        } else if self.rng.chance(chaos * chaos * 0.025 * dt) {
            c.distract = self.rng.between(1.5, 3.0);
        }
        acc += (self.rng.f32() - 0.5) * chaos * 1.5;

        // integrate
        if broken {
            acc = if c.v > 0.05 { -4.0 } else { 0.0 };
        }
        acc = acc.clamp(-9.0, a_max * 1.3);
        c.acc = acc;
        c.v = (c.v + acc * dt).max(0.0);
        let mut np = c.pos + dir * c.v * dt;
        let mut rear_hit: Option<usize> = None;
        if let Some(l) = leader {
            let o = &self.cars[l];
            let max_pos = o.pos - dir * 1.05;
            if (np - max_pos) * dir > 0.0 {
                if c.v - o.v > REAR_END_SPEED && (chaos > 0.05 || distracted || rogue) {
                    rear_hit = Some(l);
                }
                np = if (c.pos - max_pos) * dir > 0.0 { c.pos } else { max_pos };
                c.v = c.v.min(o.v);
            }
        }
        c.pos = np;
        if c.v < 0.1 {
            c.stuck += dt;
        } else {
            c.stuck = 0.0;
        }

        // execute a planned turn once we reach the turning point
        if let Some((pk, turn)) = c.plan {
            if turn != Turn::Straight && self.next_box(&c) == Some(pk) {
                let tc = self.turn_center(&c, pk, turn);
                if (c.pos - tc) * dir >= 0.0 {
                    let nd = turn_dir(c.orient, c.dir, turn);
                    let cross0 = self.lane_cross(c.orient, c.road, c.dir);
                    let old_road = c.road;
                    c.orient = c.orient.other();
                    c.road = pk;
                    c.dir = nd;
                    c.pos = cross0 as f32 + 0.5;
                    c.plan = Some((old_road, Turn::Straight));
                }
            }
        }

        // release the reservation once we have left the box
        if c.claim_n > 0 {
            let (cx, cy) = self.car_center(&c);
            let (bx, by) = c.claim_box;
            let inside = cx >= bx as f32 && cx < (bx + 2) as f32 && cy >= by as f32 && cy < (by + 2) as f32;
            if inside {
                c.claim_entered = true;
            } else if c.claim_entered {
                c.claim_n = 0;
                c.violator = false;
            }
        }

        self.cars[i] = c;

        if let Some(l) = rear_hit {
            let behind = self.cars[i].clone();
            let ahead = self.cars[l].clone();
            let (cx, cy) = self.car_center(&ahead);
            let place = self.place(cx as i32, cy as i32);
            let text = format!(
                "REAR-END: car #{}{} hit car #{} near {}",
                behind.id,
                if behind.distract > 0.0 { " (distracted)" } else { "" },
                ahead.id,
                place
            );
            self.crash(i, Some(l), text, cx as i32, cy as i32);
        }
    }

    fn cull(&mut self) {
        let mut i = 0;
        while i < self.cars.len() {
            let (pos, orient, stuck, wrecked, wreck, id) = {
                let c = &self.cars[i];
                (c.pos, c.orient, c.stuck, c.wrecked, c.wreck, c.id)
            };
            let ext = self.extent(orient);
            if pos < -1.5 || pos > ext + 1.5 {
                self.exits.push_back(self.time);
                self.exited_total += 1;
                self.cars.swap_remove(i);
            } else if wrecked && wreck <= 0.0 {
                let cb = self.cars[i].clone();
                let (cx, cy) = self.car_center(&cb);
                let place = self.place(cx as i32, cy as i32);
                self.note(LogKind::Tow, format!("Tow truck hauled wrecked car #{} away from {}", id, place), cx as i32, cy as i32);
                self.towed_total += 1;
                self.cars.swap_remove(i);
            } else if stuck > 60.0 {
                // Gridlock breaker: a tow truck removes cars that are stuck for ages.
                let cb = self.cars[i].clone();
                let (cx, cy) = self.car_center(&cb);
                let place = self.place(cx as i32, cy as i32);
                self.note(LogKind::Tow, format!("Tow truck cleared car #{} (stuck for a minute) near {}", id, place), cx as i32, cy as i32);
                self.towed_total += 1;
                self.cars.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn target_cars(&self, p: &Params) -> usize {
        let m = if self.is_active(EventKind::RushHour) { 1.7 } else { 1.0 };
        (p.cars as f32 * m).round() as usize
    }

    fn target_peds(&self, p: &Params) -> usize {
        let m = if self.is_active(EventKind::CrowdSurge) { 2.5 } else { 1.0 };
        (p.peds as f32 * m).round() as usize
    }

    fn spawn_cars(&mut self, dt: f32, p: &Params) {
        let target = self.target_cars(p);
        if self.cars.len() < target {
            self.spawn_acc += dt * 5.0;
            while self.spawn_acc >= 1.0 {
                self.spawn_acc -= 1.0;
                self.spawn_car(p, false);
                if self.cars.len() >= target {
                    break;
                }
            }
        } else {
            self.spawn_acc = 0.0;
        }
    }

    // ----------------------------------------------------------- pedestrians

    fn crossing_safe(&mut self, cwid: usize, chaos: f32) -> bool {
        // Impatient pedestrians don't look.
        let surge = self.is_active(EventKind::CrowdSurge);
        let impatient = chaos * 0.35 + if surge { 0.25 } else { 0.0 };
        if self.rng.chance(impatient) {
            if self.time >= self.near_miss_cd && self.crossing_unsafe(cwid) {
                self.near_miss_cd = self.time + 5.0;
                let (cx, cy) = {
                    let cw = &self.crosswalks[cwid];
                    match cw.orient {
                        Orient::H => (cw.k, self.hy[cw.road]),
                        Orient::V => (self.vx[cw.road], cw.k),
                    }
                };
                let place = self.place(cx, cy);
                self.note(LogKind::Violation, format!("JAYWALKING: pedestrian dashed across traffic near {}", place), cx, cy);
            }
            return true;
        }
        !self.crossing_unsafe(cwid)
    }

    /// Is a car close enough to the crosswalk that stepping out now is dangerous?
    fn crossing_unsafe(&self, cwid: usize) -> bool {
        let cw = &self.crosswalks[cwid];
        for c in &self.cars {
            if c.orient != cw.orient || c.road != cw.road {
                continue;
            }
            let near = if c.dir > 0 { cw.k as f32 } else { cw.k as f32 + 1.0 };
            let front = c.pos + 0.5 * c.dir as f32;
            let dist = (near - front) * c.dir as f32;
            let stopdist = c.v * c.v / 8.0;
            if c.v > 0.4 && dist > -1.0 && dist < stopdist + 1.6 + c.v * 0.4 {
                return true;
            }
        }
        false
    }

    fn ped_choose(&mut self, pd: &mut Ped, chaos: f32) {
        let target = if let Some(t) = pd.want {
            t
        } else {
            let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            let mut cands: Vec<((i32, i32), f32)> = Vec::new();
            for (dx, dy) in dirs {
                if !self.walkable(pd.tx + dx, pd.ty + dy) {
                    continue;
                }
                let wgt = if (dx, dy) == (-pd.dx, -pd.dy) {
                    0.05
                } else if (dx, dy) == (pd.dx, pd.dy) {
                    4.0
                } else {
                    1.5
                };
                cands.push(((dx, dy), wgt));
            }
            if cands.is_empty() {
                return;
            }
            let total: f32 = cands.iter().map(|c| c.1).sum();
            let mut r = self.rng.f32() * total;
            let mut pick = cands[0].0;
            for (d, wgt) in &cands {
                if r < *wgt {
                    pick = *d;
                    break;
                }
                r -= *wgt;
            }
            pick
        };
        let (nx, ny) = (pd.tx + target.0, pd.ty + target.1);
        let cur_cw = self.cw_at[(pd.ty * self.w + pd.tx) as usize];
        let nxt_cw = self.cw_at[(ny * self.w + nx) as usize];
        if nxt_cw >= 0 && cur_cw < 0 && !self.crossing_safe(nxt_cw as usize, chaos) {
            // wait at the kerb and look again in a moment
            pd.want = Some(target);
            pd.wait = self.rng.between(0.15, 0.5);
            return;
        }
        pd.want = None;
        pd.nx = nx;
        pd.ny = ny;
        pd.dx = target.0;
        pd.dy = target.1;
        pd.t = 0.0;
        pd.moving = true;
        if nxt_cw >= 0 {
            self.ped_occ[(ny * self.w + nx) as usize] = true;
        }
    }

    fn update_peds(&mut self, dt: f32, p: &Params) {
        let target = self.target_peds(p);
        if self.peds.len() < target {
            self.ped_acc += dt * 4.0;
            while self.ped_acc >= 1.0 {
                self.ped_acc -= 1.0;
                self.spawn_ped();
                if self.peds.len() >= target {
                    break;
                }
            }
        } else {
            self.ped_acc = 0.0;
        }
        for i in 0..self.peds.len() {
            let mut pd = self.peds[i].clone();
            if pd.moving {
                pd.t += pd.speed * dt;
                if pd.t >= 1.0 {
                    pd.tx = pd.nx;
                    pd.ty = pd.ny;
                    pd.t = 0.0;
                    pd.moving = false;
                }
            }
            if !pd.moving {
                if pd.wait > 0.0 {
                    pd.wait -= dt;
                } else {
                    self.ped_choose(&mut pd, p.chaos);
                }
            }
            self.peds[i] = pd;
        }
        // Too many pedestrians? Let a few go home (only from the sidewalk).
        if self.peds.len() > target && self.rng.chance(0.2) {
            let i = self.rng.range(self.peds.len());
            let pd = &self.peds[i];
            if !pd.moving && self.cw_at[(pd.ty * self.w + pd.tx) as usize] < 0 {
                self.peds.swap_remove(i);
            }
        }
    }

    // ----------------------------------------------------------------- stats

    fn update_stats(&mut self) {
        while let Some(&t) = self.exits.front() {
            if self.time - t > 60.0 {
                self.exits.pop_front();
            } else {
                break;
            }
        }
        let n = self.cars.len();
        let sum: f32 = self.cars.iter().map(|c| c.v).sum();
        let window = self.time.min(60.0).max(5.0);
        self.stats = Stats {
            cars: n,
            avg_kmh: if n > 0 { units_to_kmh(sum / n as f32) } else { 0.0 },
            stopped: self.cars.iter().filter(|c| c.v < 0.3).count(),
            peds: self.peds.len(),
            flow_per_min: (self.exits.len() as f64 * 60.0 / window) as f32,
            exited: self.exited_total,
            towed: self.towed_total,
            red_runs: self.red_runs,
            crashes: self.crashes,
        };
        // Gridlock watch (with hysteresis so the log doesn't flap).
        if n >= 30 {
            let frac = self.stats.stopped as f32 / n as f32;
            if !self.jam_state && frac > 0.55 {
                self.jam_state = true;
                let text = format!("GRIDLOCK: {:.0}% of cars at a standstill", frac * 100.0);
                self.note(LogKind::Info, text, -1, -1);
            } else if self.jam_state && frac < 0.35 {
                self.jam_state = false;
                self.note(LogKind::Info, "Traffic is flowing again".to_string(), -1, -1);
            }
        }
    }

    /// Screen position of a pedestrian in tile coordinates.
    pub fn ped_pos(pd: &Ped) -> (f32, f32) {
        (
            pd.tx as f32 + (pd.nx - pd.tx) as f32 * pd.t,
            pd.ty as f32 + (pd.ny - pd.ty) as f32 * pd.t,
        )
    }
}

// ------------------------------------------------- incidents, log and events

impl World {
    pub fn is_active(&self, k: EventKind) -> bool {
        self.active.iter().any(|e| e.kind == k)
    }

    /// Append a line to the traffic log.
    fn note(&mut self, kind: LogKind, text: String, x: i32, y: i32) {
        if kind != LogKind::Info && x >= 0 {
            self.last_incident = Some((x, y));
        }
        self.log.push_back(LogEntry { t: self.time, kind, text, x, y });
        while self.log.len() > LOG_CAP {
            self.log.pop_front();
        }
    }

    /// Human-readable name of the spot nearest to a tile: "C Ave & 4th St".
    pub fn place(&self, x: i32, y: i32) -> String {
        let nearest = |v: &[i32], p: i32| -> usize {
            let mut best = 0;
            for (i, s) in v.iter().enumerate() {
                if (s + 1 - p).abs() < (v[best] + 1 - p).abs() {
                    best = i;
                }
            }
            best
        };
        let i = nearest(&self.vx, x);
        let j = nearest(&self.hy, y);
        let n = j + 1;
        let suffix = match (n % 10, n % 100) {
            (_, 11..=13) => "th",
            (1, _) => "st",
            (2, _) => "nd",
            (3, _) => "rd",
            _ => "th",
        };
        format!("{} Ave & {}{} St", (b'A' + (i % 26) as u8) as char, n, suffix)
    }

    /// Two cars (or one, when `b` is None) are wrecked.
    fn crash(&mut self, a: usize, b: Option<usize>, text: String, x: i32, y: i32) {
        let dur_a = self.rng.between(16.0, 26.0);
        wreck_car(&mut self.cars[a], dur_a);
        if let Some(b) = b {
            let dur_b = self.rng.between(16.0, 26.0);
            wreck_car(&mut self.cars[b], dur_b);
        }
        self.crashes += 1;
        self.note(LogKind::Crash, text, x, y);
    }

    /// Cars from different roads that end up overlapping inside a junction crash.
    fn detect_collisions(&mut self) {
        let mut cand: Vec<(usize, f32, f32)> = Vec::new();
        for (i, c) in self.cars.iter().enumerate() {
            let (cx, cy) = self.car_center(c);
            if cx < 0.0 || cy < 0.0 || cx >= self.w as f32 || cy >= self.h as f32 {
                continue;
            }
            if self.tile(cx as i32, cy as i32) == Tile::Box {
                cand.push((i, cx, cy));
            }
        }
        for a in 0..cand.len() {
            for b in (a + 1)..cand.len() {
                let (i, ax, ay) = cand[a];
                let (j, bx, by) = cand[b];
                let (ca, cb) = (&self.cars[i], &self.cars[j]);
                if ca.wrecked && cb.wrecked {
                    continue;
                }
                if ca.orient == cb.orient && ca.dir == cb.dir {
                    continue; // same lane: the rear-end logic handles those
                }
                if (ax - bx).abs() < 0.9 && (ay - by).abs() < 0.9 {
                    let (id_a, id_b) = (ca.id, cb.id);
                    let culprit = if ca.violator { Some(id_a) } else if cb.violator { Some(id_b) } else { None };
                    let place = self.place(ax as i32, ay as i32);
                    let text = match culprit {
                        Some(id) => format!("CRASH: car #{} and car #{} collided at {} (car #{} ran the red)", id_a, id_b, place, id),
                        None => format!("CRASH: car #{} and car #{} collided at {}", id_a, id_b, place),
                    };
                    self.crash(i, Some(j), text, ax as i32, ay as i32);
                }
            }
        }
    }

    /// Cars hitting an oil slick spin out and become wrecks.
    fn update_hazards(&mut self) {
        let now = self.time;
        self.hazards.retain(|h| h.until > now);
        if self.hazards.is_empty() {
            return;
        }
        for i in 0..self.cars.len() {
            if self.cars[i].wrecked {
                continue;
            }
            let (cx, cy) = self.car_center(&self.cars[i]);
            let (tx, ty) = (cx.floor() as i32, cy.floor() as i32);
            let hit = self.hazards.iter().position(|h| h.x == tx && h.y == ty);
            if let Some(hi) = hit {
                if self.cars[i].v > 1.5 && self.rng.chance(0.6) {
                    let id = self.cars[i].id;
                    let place = self.place(tx, ty);
                    self.hazards[hi].hits += 1;
                    self.crash(i, None, format!("SPIN-OUT: car #{} skidded on oil near {}", id, place), tx, ty);
                }
            }
        }
    }

    // ------------------------------------------------------------- events

    /// Make the city throw something at you right now.
    pub fn trigger_event(&mut self) {
        self.next_event = self.time;
        self.event_now = true;
    }

    fn start_event(&mut self, kind: EventKind, secs: f64) {
        let until = self.time + secs;
        self.active.push(ActiveEvent { kind, until });
    }

    /// The city's "mind of its own": every so often something happens.
    fn run_events(&mut self, p: &Params) {
        // expire
        let now = self.time;
        let mut ended: Vec<EventKind> = Vec::new();
        self.active.retain(|e| {
            if e.until <= now {
                ended.push(e.kind);
                false
            } else {
                true
            }
        });
        for k in ended {
            if k == EventKind::CrowdSurge {
                self.hotspot = None;
            }
            self.note(LogKind::Event, format!("{} is over", k.label()), -1, -1);
        }

        let forced = std::mem::take(&mut self.event_now);
        if !forced && (p.events <= 0.001 || self.time < self.next_event) {
            return;
        }
        let gap = self.rng.between(14.0, 34.0) as f64 / (0.25 + 1.5 * p.events as f64);
        self.next_event = self.time + gap;

        match self.rng.range(8) {
            0 if !self.is_active(EventKind::RushHour) => {
                self.start_event(EventKind::RushHour, 50.0);
                self.note(LogKind::Event, "RUSH HOUR: commuters flood the roads".to_string(), -1, -1);
            }
            1 if !self.is_active(EventKind::Rain) => {
                self.start_event(EventKind::Rain, 45.0);
                self.note(LogKind::Event, "RAIN: slick roads, slower and twitchier drivers".to_string(), -1, -1);
            }
            2 if !self.is_active(EventKind::CrowdSurge) => {
                // a concert / match lets out somewhere on a sidewalk
                for _ in 0..40 {
                    let x = self.rng.range(self.w as usize) as i32;
                    let y = self.rng.range(self.h as usize) as i32;
                    if self.tile(x, y) == Tile::Sidewalk {
                        self.hotspot = Some((x, y));
                        break;
                    }
                }
                if let Some((x, y)) = self.hotspot {
                    self.start_event(EventKind::CrowdSurge, 30.0);
                    let place = self.place(x, y);
                    self.note(LogKind::Event, format!("CROWD SURGE: a big crowd spills onto the streets near {}", place), x, y);
                }
            }
            3 if !self.is_active(EventKind::PoliceChase) && !self.cars.is_empty() => {
                let i = self.rng.range(self.cars.len());
                if !self.cars[i].wrecked {
                    self.cars[i].rogue = 25.0;
                    self.cars[i].aggr = 1.0;
                    let id = self.cars[i].id;
                    self.start_event(EventKind::PoliceChase, 25.0);
                    let (cx, cy) = self.car_center(&self.cars[i].clone());
                    let place = self.place(cx as i32, cy as i32);
                    self.note(LogKind::Event, format!("POLICE CHASE: car #{} is fleeing at high speed near {}", id, place), cx as i32, cy as i32);
                }
            }
            4 if !self.is_active(EventKind::Outage) => {
                // power cut across a few neighbouring junctions
                let b = self.rng.range(self.boxes.len());
                let (ox, oy) = (self.boxes[b].x0, self.boxes[b].y0);
                let mut count = 0;
                let until = self.time + 28.0;
                for bx in self.boxes.iter_mut() {
                    if (bx.x0 - ox).abs() < 36 && (bx.y0 - oy).abs() < 26 {
                        bx.dark_until = until;
                        count += 1;
                    }
                }
                self.start_event(EventKind::Outage, 28.0);
                let place = self.place(ox, oy);
                self.note(LogKind::Event, format!("POWER OUTAGE: {} signals dark around {} (flashing yellow)", count, place), ox, oy);
            }
            5 if !self.cars.is_empty() => {
                // plain old breakdown
                let i = self.rng.range(self.cars.len());
                if !self.cars[i].wrecked && self.cars[i].broken <= 0.0 {
                    self.cars[i].broken = self.rng.between(12.0, 22.0);
                    let id = self.cars[i].id;
                    let (cx, cy) = self.car_center(&self.cars[i].clone());
                    let place = self.place(cx as i32, cy as i32);
                    self.note(LogKind::Event, format!("BREAKDOWN: car #{} stalled near {}", id, place), cx as i32, cy as i32);
                }
            }
            6 => {
                // oil spill on a random stretch of road
                for _ in 0..60 {
                    let x = self.rng.range(self.w as usize) as i32;
                    let y = self.rng.range(self.h as usize) as i32;
                    if let Tile::Road(o) = self.tile(x, y) {
                        let until = self.time + 40.0;
                        for k in 0..3 {
                            let (hx, hy) = if o == Orient::H { (x + k, y) } else { (x, y + k) };
                            if hx < self.w && hy < self.h && matches!(self.tile(hx, hy), Tile::Road(_)) {
                                self.hazards.push(Hazard { x: hx, y: hy, until, hits: 0 });
                            }
                        }
                        let place = self.place(x, y);
                        self.note(LogKind::Event, format!("OIL SPILL near {}: cars may skid", place), x, y);
                        break;
                    }
                }
            }
            _ => {}
        }
    }
}
