//! traffic-tui: a lively ASCII traffic simulator for your terminal.
//!
//!   cargo run --release
//!   cargo run --release -- --snapshot 30     # headless: simulate 30s, print one frame

mod ui;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use std::io;
use std::time::{Duration, Instant};
use traffic::app::{App, SLIDERS};

const FRAME: Duration = Duration::from_millis(33); // ~30 FPS

/// Size of the generated city, in roads. The terminal shows a scrollable window onto it.
const ROADS_X: usize = 14;
const ROADS_Y: usize = 10;

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("traffic-tui - ASCII traffic simulator\n\nUSAGE:\n  traffic-tui              run the interactive simulator\n  traffic-tui --snapshot N [--cars C --peds P --chaos X --quiet 1]\n                           headless: simulate N seconds, print one frame + stats");
        return Ok(());
    }
    if let Some(pos) = args.iter().position(|a| a == "--snapshot") {
        let secs: f32 = args.get(pos + 1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
        let opt = |name: &str, d: f32| -> f32 {
            args.iter()
                .position(|a| a == name)
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(d)
        };
        snapshot(secs, opt("--cars", 110.0), opt("--peds", 40.0), opt("--chaos", 25.0), opt("--quiet", 0.0) > 0.0);
        return Ok(());
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42)
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    let mut app = App::new(ROADS_X, ROADS_Y, seed());
    let size = terminal.size()?;
    let (vc, vr) = map_view(size.width, size.height);
    app.set_view(vc, vr);
    app.center();

    let mut last = Instant::now();
    let mut next_frame = Instant::now();
    loop {
        // the window follows the terminal size
        let size = terminal.size()?;
        let (vc, vr) = map_view(size.width, size.height);
        app.set_view(vc, vr);

        terminal.draw(|f| ui::draw(f, &app))?;

        // handle input until it is time for the next frame
        loop {
            let timeout = next_frame.saturating_duration_since(Instant::now());
            if !event::poll(timeout)? {
                break;
            }
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let shift = key.modifiers.contains(KeyModifiers::SHIFT);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
                    KeyCode::Up | KeyCode::Char('k') => app.select(-1),
                    KeyCode::Down | KeyCode::Char('j') => app.select(1),
                    KeyCode::Left | KeyCode::Char('h') => app.adjust(-1, shift),
                    KeyCode::Right | KeyCode::Char('l') => app.adjust(1, shift),
                    KeyCode::Char('H') => app.adjust(-1, true),
                    KeyCode::Char('L') => app.adjust(1, true),
                    // scroll the map: w/a/s/d, capitals scroll 4x faster
                    KeyCode::Char(ch @ ('w' | 'W' | 'a' | 'A' | 's' | 'S' | 'd' | 'D')) => {
                        let k = if ch.is_ascii_uppercase() { 4 } else { 1 };
                        match ch.to_ascii_lowercase() {
                            'w' => app.scroll(0, -3 * k),
                            's' => app.scroll(0, 3 * k),
                            'a' => app.scroll(-6 * k, 0),
                            _ => app.scroll(6 * k, 0),
                        }
                    }
                    KeyCode::Char('c') => app.center(),
                    KeyCode::Char('g') => app.goto_incident(),
                    KeyCode::Char('e') => app.world.trigger_event(),
                    KeyCode::Char(' ') | KeyCode::Char('p') => app.paused = !app.paused,
                    KeyCode::Char('r') => {
                        let (nx, ny) = (app.world.n_x, app.world.n_y);
                        app.reset(nx, ny);
                    }
                    KeyCode::Char(d @ '1'..='7') => {
                        let i = d as usize - '1' as usize;
                        if i < SLIDERS.len() {
                            app.sel = i;
                        }
                    }
                    _ => {}
                }
            }
        }

        let now = Instant::now();
        app.update(now.duration_since(last).as_secs_f32());
        last = now;
        next_frame = now + FRAME;
    }
}

/// Map window in character cells, after borders and the sidebar.
fn map_view(term_w: u16, term_h: u16) -> (usize, usize) {
    let cols = (term_w as i32 - ui::SIDEBAR_W as i32 - 2).max(0);
    let rows = (term_h as i32 - 2 - ui::LOG_H as i32).max(0);
    (cols as usize, rows as usize)
}

fn snapshot(secs: f32, cars: f32, peds: f32, chaos: f32, quiet: bool) {
    let mut app = App::new(5, 4, 7);
    app.set_value(0, cars);
    app.set_value(2, peds);
    app.set_value(1, chaos);
    app.reset(5, 4);
    let steps = (secs / 0.033) as usize;
    for _ in 0..steps {
        app.update(0.033);
    }
    if !quiet {
        println!("{}", app.frame().to_text());
    }
    let s = &app.world.stats;
    println!(
        "map={}x{} tiles  t={:.0}s cars={} avg={:.1}km/h stopped={} peds={} flow={:.0}/min exited={} towed={} red_runs={} crashes={}",
        app.world.w, app.world.h, app.world.time, s.cars, s.avg_kmh, s.stopped, s.peds, s.flow_per_min, s.exited, s.towed, s.red_runs, s.crashes
    );
    println!("--- traffic log (last 25) ---");
    let n = app.world.log.len();
    for e in app.world.log.iter().skip(n.saturating_sub(25)) {
        println!("{}", e.line());
    }
}
