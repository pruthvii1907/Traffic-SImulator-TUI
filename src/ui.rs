//! Ratatui drawing code: the city viewport on the left, the control panel on the right.
//! Everything drawn is plain ASCII (borders included). Palette: dark + white.

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Paragraph, Widget},
    Frame,
};
use traffic::app::{sparkline, App, SLIDERS};
use traffic::render::{light_color, Grid, Rgb, CAR_PALETTE, GREEN, RED, YELLOW};
use traffic::sim::{Light, LogKind};

pub const SIDEBAR_W: u16 = 42;
/// Height of the traffic-log strip under the map (borders included).
pub const LOG_H: u16 = 10;

const WHITE: Color = Color::Rgb(235, 235, 238);
const GREY: Color = Color::Rgb(150, 150, 158);
const DARK: Color = Color::Rgb(90, 90, 98);
const AMBER: Color = Color::Rgb(205, 175, 70);
const ALERT: Color = Color::Rgb(215, 85, 85);
const CAR_GREEN: Color = Color::Rgb(46, 139, 87);

fn col(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

fn ascii_block<'a>(title: &'a str, color: Color) -> Block<'a> {
    Block::bordered()
        .border_set(border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        })
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color).add_modifier(Modifier::BOLD)))
}

/// Blits the pre-rendered city window into the terminal buffer, centred in `area`.
struct MapWidget<'a>(&'a Grid);

impl Widget for MapWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let g = self.0;
        let ox = area.x + area.width.saturating_sub(g.cols as u16) / 2;
        let oy = area.y + area.height.saturating_sub(g.rows as u16) / 2;
        for y in 0..g.rows.min(area.height as usize) {
            for x in 0..g.cols.min(area.width as usize) {
                let c = g.cells[y * g.cols + x];
                let cell = &mut buf[(ox + x as u16, oy + y as u16)];
                cell.set_char(c.ch);
                cell.set_fg(col(c.fg));
                cell.set_bg(col(c.bg));
                cell.modifier = if c.bold { Modifier::BOLD } else { Modifier::empty() };
            }
        }
    }
}

fn slider_line(app: &App, i: usize) -> Line<'static> {
    let selected = i == app.sel;
    let hue = if selected { WHITE } else { GREY };
    let track = 14usize;
    let pos = (app.frac(i) * (track - 1) as f32).round() as usize;
    let mut bar = String::from("[");
    for k in 0..track {
        bar.push(if k < pos {
            '='
        } else if k == pos {
            'O'
        } else {
            '-'
        });
    }
    bar.push(']');
    let name_style = if selected {
        Style::default().fg(WHITE).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(GREY)
    };
    Line::from(vec![
        Span::styled(if selected { "> " } else { "  " }, Style::default().fg(CAR_GREEN).add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<12}", SLIDERS[i].name), name_style),
        Span::styled(bar, Style::default().fg(if selected { CAR_GREEN } else { DARK })),
        Span::styled(
            format!("{:>8}", app.fmt_value(i)),
            Style::default().fg(hue).add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() }),
        ),
    ])
}

fn stat(label: &str, value: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {:<15}", label), Style::default().fg(GREY)),
        Span::styled(value, Style::default().fg(color).add_modifier(Modifier::BOLD)),
    ])
}

fn heading(t: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!(" -- {} {}", t, "-".repeat(34usize.saturating_sub(t.len()))),
        Style::default().fg(DARK),
    ))
}

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::horizontal([Constraint::Min(10), Constraint::Length(SIDEBAR_W)]).split(area);
    let left = Layout::vertical([Constraint::Min(4), Constraint::Length(LOG_H)]).split(chunks[0]);

    // ----- city
    let title = format!(
        " TRAFFIC.SIM{}  view {},{} / {}x{} ",
        if app.paused { " [PAUSED]" } else { "" },
        app.cam_x / 2,
        app.cam_y,
        app.world.w,
        app.world.h
    );
    let block = ascii_block(&title, WHITE);
    let inner = block.inner(left[0]);
    f.render_widget(block, left[0]);
    f.render_widget(MapWidget(&app.frame()), inner);

    // ----- traffic log
    draw_log(f, app, left[1]);

    // ----- sidebar
    let block = ascii_block(" CONTROLS ", GREY);
    let inner = block.inner(chunks[1]);
    f.render_widget(block, chunks[1]);

    let s = &app.world.stats;
    let jam_pct = if s.cars > 0 { s.stopped * 100 / s.cars } else { 0 };
    let jam_color = match jam_pct {
        0..=15 => WHITE,
        16..=35 => GREY,
        _ => Color::Rgb(210, 90, 90),
    };
    let width = (inner.width as usize).saturating_sub(2);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::raw(""));
    for i in 0..SLIDERS.len() {
        lines.push(slider_line(app, i));
    }
    lines.push(Line::raw(""));
    lines.push(heading("LIVE STATS"));
    lines.push(stat("Cars on road", format!("{}", s.cars), WHITE));
    lines.push(stat("Avg speed", format!("{:.0} km/h", s.avg_kmh), WHITE));
    lines.push(stat("Jammed cars", format!("{} ({}%)", s.stopped, jam_pct), jam_color));
    lines.push(stat("Pedestrians", format!("{}", s.peds), WHITE));
    lines.push(stat("Flow", format!("{:.0} cars/min", s.flow_per_min), WHITE));
    lines.push(stat("Red-light runs", format!("{}", s.red_runs), if s.red_runs > 0 { AMBER } else { DARK }));
    lines.push(stat("Crashes", format!("{}", s.crashes), if s.crashes > 0 { ALERT } else { DARK }));
    lines.push(stat("Cars towed", format!("{}", s.towed), DARK));
    lines.push(Line::raw(""));
    lines.push(heading("CITY EVENTS"));
    if app.world.active.is_empty() {
        lines.push(Line::from(Span::styled(" all quiet...", Style::default().fg(DARK))));
    }
    for e in app.world.active.iter().take(3) {
        let left_s = (e.until - app.world.time).max(0.0);
        lines.push(Line::from(vec![
            Span::styled(format!(" {:<14}", e.kind.label()), Style::default().fg(WHITE).add_modifier(Modifier::BOLD)),
            Span::styled(format!("{:>3.0}s left", left_s), Style::default().fg(GREY)),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(" avg speed", Style::default().fg(GREY))));
    lines.push(Line::from(Span::styled(
        format!(" {}", sparkline(&app.speed_hist, width)),
        Style::default().fg(CAR_GREEN),
    )));
    lines.push(Line::from(Span::styled(" jam level", Style::default().fg(GREY))));
    lines.push(Line::from(Span::styled(
        format!(" {}", sparkline(&app.jam_hist, width)),
        Style::default().fg(WHITE),
    )));
    lines.push(Line::raw(""));
    lines.push(heading("LEGEND"));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("[> ", Style::default().fg(col(CAR_PALETTE[0])).add_modifier(Modifier::BOLD)),
        Span::styled("<] ", Style::default().fg(col(CAR_PALETTE[2])).add_modifier(Modifier::BOLD)),
        Span::styled("/\\ ", Style::default().fg(col(CAR_PALETTE[3])).add_modifier(Modifier::BOLD)),
        Span::styled("\\/ ", Style::default().fg(col(CAR_PALETTE[5])).add_modifier(Modifier::BOLD)),
        Span::styled("cars ", Style::default().fg(GREY)),
        Span::styled("red bg", Style::default().bg(Color::Rgb(64, 14, 14)).fg(WHITE)),
        Span::styled("=braking", Style::default().fg(GREY)),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("o", Style::default().fg(WHITE).add_modifier(Modifier::BOLD)),
        Span::styled(" pedestrian   ", Style::default().fg(GREY)),
        Span::styled("o", Style::default().fg(col(light_color(Light::Green))).add_modifier(Modifier::BOLD)),
        Span::styled("o", Style::default().fg(col(light_color(Light::Red))).add_modifier(Modifier::BOLD)),
        Span::styled(" N/S|E/W light", Style::default().fg(GREY)),
    ]));
    lines.push(Line::from(vec![
        Span::raw(" "),
        Span::styled("=== ", Style::default().fg(WHITE)),
        Span::styled("zebra   ", Style::default().fg(GREY)),
        Span::styled("go ", Style::default().fg(col(GREEN))),
        Span::styled("slow ", Style::default().fg(col(YELLOW))),
        Span::styled("stop", Style::default().fg(col(RED))),
    ]));
    lines.push(Line::raw(""));
    lines.push(heading("KEYS"));
    for k in [
        " up/down  j/k  pick a slider (or 1-7)",
        " left/right h/l  turn the knob",
        " shift or H/L   big steps",
        " w a s d  scroll map  (WASD = fast)",
        " g go to last incident   e trigger event",
        " c center  space pause  r new  q quit",
    ] {
        lines.push(Line::from(Span::styled(k, Style::default().fg(GREY))));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

fn log_color(k: LogKind) -> Color {
    match k {
        LogKind::Info => GREY,
        LogKind::Violation => AMBER,
        LogKind::Crash => ALERT,
        LogKind::Event => WHITE,
        LogKind::Tow => DARK,
    }
}

fn log_tag(k: LogKind) -> &'static str {
    match k {
        LogKind::Info => "  ",
        LogKind::Violation => "! ",
        LogKind::Crash => "X ",
        LogKind::Event => "* ",
        LogKind::Tow => "~ ",
    }
}

/// The scrolling traffic log: newest entry at the bottom.
fn draw_log(f: &mut Frame, app: &App, area: Rect) {
    let block = ascii_block(" TRAFFIC LOG  (! violation  X crash  * event) ", GREY);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let rows = inner.height as usize;
    let width = inner.width as usize;
    let log = &app.world.log;
    let skip = log.len().saturating_sub(rows);
    let mut lines: Vec<Line> = Vec::new();
    if log.is_empty() {
        lines.push(Line::from(Span::styled(" nothing to report yet...", Style::default().fg(DARK))));
    }
    for e in log.iter().skip(skip) {
        let mut text = format!("{}{}", log_tag(e.kind), e.line());
        if text.len() > width.saturating_sub(1) {
            text.truncate(width.saturating_sub(4));
            text.push_str("...");
        }
        let mut st = Style::default().fg(log_color(e.kind));
        if matches!(e.kind, LogKind::Crash | LogKind::Event) {
            st = st.add_modifier(Modifier::BOLD);
        }
        lines.push(Line::from(Span::styled(text, st)));
    }
    f.render_widget(Paragraph::new(lines), inner);
}
