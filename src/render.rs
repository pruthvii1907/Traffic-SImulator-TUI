//! Turns a `World` into a grid of ASCII cells. UI-agnostic: the terminal front
//! end (ratatui) and the browser front end (canvas) both consume this grid.
//!
//! Every glyph is plain ASCII. Colours are 24-bit RGB, deliberately minimal:
//! near-black roads, grey sidewalks and buildings, white pedestrians, dark
//! green cars.

use crate::sim::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rgb(pub u8, pub u8, pub u8);

#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub bold: bool,
}

#[derive(Clone)]
pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize, fill: Cell) -> Grid {
        Grid { cols, rows, cells: vec![fill; cols * rows] }
    }
    pub fn set(&mut self, x: i32, y: i32, ch: char, fg: Rgb, bold: bool) {
        if x < 0 || y < 0 || x as usize >= self.cols || y as usize >= self.rows {
            return;
        }
        let c = &mut self.cells[y as usize * self.cols + x as usize];
        c.ch = ch;
        c.fg = fg;
        c.bold = bold;
    }
    pub fn set_bg(&mut self, x: i32, y: i32, bg: Rgb) {
        if x < 0 || y < 0 || x as usize >= self.cols || y as usize >= self.rows {
            return;
        }
        self.cells[y as usize * self.cols + x as usize].bg = bg;
    }
    /// Plain-text dump (no colours), handy for tests and `--snapshot`.
    pub fn to_text(&self) -> String {
        let mut s = String::with_capacity((self.cols + 1) * self.rows);
        for y in 0..self.rows {
            for x in 0..self.cols {
                s.push(self.cells[y * self.cols + x].ch);
            }
            s.push('\n');
        }
        s
    }
}

pub const ROAD_BG: Rgb = Rgb(12, 12, 14);
pub const SIDEWALK_BG: Rgb = Rgb(36, 36, 40);
const BUILDING_BG: Rgb = Rgb(22, 22, 25);
const PARK_BG: Rgb = Rgb(17, 19, 17);
const BRAKE_BG: Rgb = Rgb(64, 14, 14);

/// Dark greens, from near-black forest to muted sea green.
pub const CAR_PALETTE: [Rgb; CAR_COLORS] = [
    Rgb(28, 125, 66),
    Rgb(22, 105, 72),
    Rgb(46, 139, 87),
    Rgb(34, 110, 50),
    Rgb(60, 130, 60),
    Rgb(26, 92, 60),
    Rgb(52, 120, 85),
    Rgb(38, 140, 70),
    Rgb(18, 100, 80),
    Rgb(70, 125, 50),
    Rgb(30, 115, 90),
    Rgb(44, 98, 44),
];
const PED_PALETTE: [Rgb; PED_COLORS] = [
    Rgb(240, 240, 240),
    Rgb(215, 215, 220),
    Rgb(190, 190, 196),
    Rgb(250, 250, 250),
    Rgb(165, 165, 172),
];

// Muted so they stay small accents on the monochrome map.
pub const GREEN: Rgb = Rgb(70, 190, 100);
pub const YELLOW: Rgb = Rgb(200, 170, 60);
pub const RED: Rgb = Rgb(200, 60, 60);

pub fn light_color(l: Light) -> Rgb {
    match l {
        Light::Green => GREEN,
        Light::Yellow => YELLOW,
        Light::Red => RED,
    }
}

fn hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E3779B1) ^ (y as u32).wrapping_mul(0x85EBCA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B3C6D);
    h ^= h >> 12;
    h
}

/// The static city (roads, sidewalks, buildings). Build once, reuse every frame.
pub fn build_base(world: &World) -> Grid {
    let mut g = Grid::new(
        world.w as usize * 2,
        world.h as usize,
        Cell { ch: ' ', fg: Rgb(0, 0, 0), bg: ROAD_BG, bold: false },
    );
    let dim = Rgb(52, 52, 60);
    let zebra = Rgb(215, 215, 220);
    for y in 0..world.h {
        for x in 0..world.w {
            let (c0, c1, fg, bg) = match world.tile(x, y) {
                Tile::Sidewalk => (' ', ' ', dim, SIDEWALK_BG),
                Tile::Building(5) => {
                    const TREES: [(char, char); 6] =
                        [('&', '&'), ('*', ' '), (' ', '&'), ('"', '"'), (' ', ' '), ('.', '*')];
                    let (a, b) = TREES[(hash(x, y) % 6) as usize];
                    (a, b, Rgb(70, 84, 74), PARK_BG)
                }
                Tile::Building(s) => {
                    let (a, b, fg) = match s {
                        0 => ('[', ']', Rgb(98, 98, 108)),
                        1 => {
                            if (x + y) % 2 == 0 {
                                ('%', '%', Rgb(88, 88, 96))
                            } else {
                                ('#', '#', Rgb(76, 76, 84))
                            }
                        }
                        2 => (':', ':', Rgb(72, 72, 80)),
                        3 => ('+', '+', Rgb(104, 104, 112)),
                        4 => ('(', ')', Rgb(84, 84, 94)),
                        _ => ('#', '#', Rgb(176, 176, 184)), // tower: the bright white one
                    };
                    (a, b, fg, BUILDING_BG)
                }
                Tile::Road(Orient::H) => {
                    // faint direction arrows: lower row heads East, upper row West
                    if x % 4 == 1 {
                        if world.is_second_lane(Orient::H, x, y) {
                            ('>', ' ', dim, ROAD_BG)
                        } else {
                            ('<', ' ', dim, ROAD_BG)
                        }
                    } else {
                        (' ', ' ', dim, ROAD_BG)
                    }
                }
                Tile::Road(Orient::V) => {
                    if y % 4 == 1 {
                        if !world.is_second_lane(Orient::V, x, y) {
                            ('v', ' ', dim, ROAD_BG)
                        } else {
                            (' ', '^', dim, ROAD_BG)
                        }
                    } else {
                        (' ', ' ', dim, ROAD_BG)
                    }
                }
                Tile::Box => (' ', ' ', dim, ROAD_BG),
                Tile::Crosswalk(Orient::H) => ('=', '=', zebra, ROAD_BG),
                Tile::Crosswalk(Orient::V) => ('|', '|', zebra, ROAD_BG),
            };
            let (cx, cy) = (x * 2, y);
            g.set(cx, cy, c0, fg, false);
            g.set(cx + 1, cy, c1, fg, false);
            g.set_bg(cx, cy, bg);
            g.set_bg(cx + 1, cy, bg);
        }
    }
    g
}

/// The visible window of the city: a `vcols` x `vrows` slice of the static
/// base starting at (`cam_x`, `cam_y`) in character cells, plus traffic
/// lights, pedestrians and cars drawn on top. The window is clamped to the map.
pub fn render(world: &World, base: &Grid, cam_x: usize, cam_y: usize, vcols: usize, vrows: usize) -> Grid {
    let cols = vcols.min(base.cols);
    let rows = vrows.min(base.rows);
    let cam_x = cam_x.min(base.cols - cols);
    let cam_y = cam_y.min(base.rows - rows);

    let mut g = Grid { cols, rows, cells: Vec::with_capacity(cols * rows) };
    for y in 0..rows {
        let s = (y + cam_y) * base.cols + cam_x;
        g.cells.extend_from_slice(&base.cells[s..s + cols]);
    }
    let (ox, oy) = (cam_x as i32, cam_y as i32);

    // traffic lights: left dot = North/South signal, right dot = East/West signal
    for (b, bx) in world.boxes.iter().enumerate() {
        if world.time < bx.dark_until {
            // outage: both lamps blink yellow together
            let on = (world.time * 2.0) as i64 % 2 == 0;
            let c = if on { YELLOW } else { Rgb(70, 62, 28) };
            g.set(bx.x0 * 2 + 1 - ox, bx.y0 - oy, 'x', c, true);
            g.set(bx.x0 * 2 + 2 - ox, bx.y0 - oy, 'x', c, true);
            continue;
        }
        let ns = light_color(world.light(b, Orient::V));
        let ew = light_color(world.light(b, Orient::H));
        g.set(bx.x0 * 2 + 1 - ox, bx.y0 - oy, 'o', ns, true);
        g.set(bx.x0 * 2 + 2 - ox, bx.y0 - oy, 'o', ew, true);
    }

    // rain: faint streaks on empty cells
    if world.is_active(EventKind::Rain) {
        let frame = (world.time * 9.0) as i32;
        for y in 0..rows {
            for x in 0..cols {
                let h = hash(x as i32 + cam_x as i32, y as i32 * 31 + frame * 7);
                if h % 23 == 0 {
                    let c = &mut g.cells[y * cols + x];
                    if c.ch == ' ' {
                        c.ch = if h % 2 == 0 { '\'' } else { '`' };
                        c.fg = Rgb(100, 105, 120);
                        c.bold = false;
                    }
                }
            }
        }
    }

    // oil slicks
    for h in &world.hazards {
        for k in 0..2 {
            g.set(h.x * 2 + k - ox, h.y - oy, '~', Rgb(120, 120, 135), true);
            g.set_bg(h.x * 2 + k - ox, h.y - oy, Rgb(30, 30, 40));
        }
    }

    // pedestrians
    for pd in &world.peds {
        let (xf, yf) = World::ped_pos(pd);
        let col = (xf * 2.0 + 0.5).floor() as i32;
        let row = (yf + 0.5).floor() as i32;
        g.set(col - ox, row - oy, 'o', PED_PALETTE[pd.color as usize % PED_COLORS], true);
    }

    // cars
    for c in &world.cars {
        let glyph = match (c.orient, c.dir > 0) {
            (Orient::H, true) => ['[', '>'],
            (Orient::H, false) => ['<', ']'],
            (Orient::V, true) => ['\\', '/'],
            (Orient::V, false) => ['/', '\\'],
        };
        let cross = world.lane_cross(c.orient, c.road, c.dir);
        let (col, row) = match c.orient {
            Orient::H => (((c.pos - 0.5) * 2.0).round() as i32, cross),
            Orient::V => (cross * 2, c.pos.floor() as i32),
        };
        let (col, row) = (col - ox, row - oy);
        if row < 0 || row >= rows as i32 || col < -2 || col >= cols as i32 {
            continue;
        }
        let mut fg = CAR_PALETTE[c.color as usize % CAR_COLORS];
        let braking = c.acc < -1.5 || c.v < 0.2;
        let mut glyph = glyph;
        let mut bg_override: Option<Rgb> = None;
        if c.wrecked {
            glyph = ['X', 'X'];
            fg = Rgb(235, 235, 238);
            bg_override = Some(Rgb(110, 20, 20));
        } else if c.broken > 0.0 {
            // hazard lights flash
            glyph = ['!', '!'];
            fg = if (world.time * 3.0) as i64 % 2 == 0 { YELLOW } else { Rgb(90, 80, 35) };
        } else if c.rogue > 0.0 {
            // police chase target: bright white, flashing background
            fg = Rgb(245, 245, 250);
            bg_override = Some(if (world.time * 6.0) as i64 % 2 == 0 { Rgb(110, 20, 20) } else { Rgb(25, 30, 90) });
        }
        for (k, ch) in glyph.iter().enumerate() {
            g.set(col + k as i32, row, *ch, fg, true);
            if let Some(b) = bg_override {
                g.set_bg(col + k as i32, row, b);
            } else if braking {
                g.set_bg(col + k as i32, row, BRAKE_BG);
            }
        }
    }
    g
}
