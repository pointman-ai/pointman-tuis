//! What every TUI here shares (EXT-27; James, Pointman thread 17421): one look and one set of keys.
//!
//! A TUI says what it shows ([`Tui`]): its screens, how to load its data, and how to draw each screen. The kit does
//! the rest:
//! - the terminal, and the frame: a top bar with the title and how fresh the data is, tabs, and the keys at the bottom;
//! - loading on a background thread, every few seconds and on `r`, keeping the last data when a load fails;
//! - the keys every TUI shares (README.md): `tab` screens, `↑↓`/`jk` move, `Enter` open, `Esc` back, `r` refresh,
//!   `?` help, `q` quit;
//! - `--snapshot`, which prints one frame as text (`--size 80x24`, `--screen N`, `--row N`, `--open`), so an agent
//!   or a test can check a layout without a terminal.
//!
//! TUIs only read. One that changes something asks first, on screen.

use std::io::IsTerminal;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{bail, Context, Result};
pub use ratatui;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Tabs, Wrap};
use ratatui::{Frame, Terminal};

/// A TUI: what it shows and how.
pub trait Tui {
    /// Everything it shows, as one load reads it.
    type Data: Send + 'static;

    /// What it shows, for the top bar: "Releases".
    fn title(&self) -> String;

    /// Its screens, in tab order. One is fine: then there are no tabs.
    fn screens(&self) -> Vec<String>;

    /// Reads everything fresh. It runs on a background thread, so it may take a few seconds; an error keeps the last
    /// data on screen and says why at the top.
    fn loader(&self) -> Box<dyn Fn() -> Result<Self::Data> + Send>;

    /// How often to load again.
    fn every(&self) -> Duration {
        Duration::from_secs(10)
    }

    /// How many rows the list on `screen` has: `↑↓` move among them and `Enter` opens one. 0 for a screen without a
    /// list.
    fn rows(&self, data: &Self::Data, screen: usize) -> usize;

    /// Draws the screen the person is on (`at`) into `area`.
    fn draw(&self, f: &mut Frame, area: Rect, data: &Self::Data, at: &At);

    /// Its own keys, for help: (keys, what they do).
    fn keys(&self) -> Vec<(String, String)> {
        vec![]
    }
}

/// Where the person is: the screen, the row picked in its list, and whether that row is open (`Enter`; `Esc` closes).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct At {
    pub screen: usize,
    pub row: usize,
    pub open: bool,
}

/// What a key asks for next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Stay,
    Refresh,
    Quit,
}

/// How fresh the data is.
#[derive(Clone, Debug, PartialEq)]
enum News {
    Loading,
    Updated(SystemTime),
    /// The last load failed: why, in one line. The data from before stays.
    Failed(String),
}

/// A TUI on screen: its data, where the person is, and whether help is up.
pub struct App<T: Tui> {
    tui: T,
    data: Option<T::Data>,
    at: At,
    help: bool,
    news: News,
}

impl<T: Tui> App<T> {
    pub fn new(tui: T) -> Self {
        App { tui, data: None, at: At::default(), help: false, news: News::Loading }
    }

    pub fn at(&self) -> At {
        self.at
    }

    pub fn data(&self) -> Option<&T::Data> {
        self.data.as_ref()
    }

    /// What a load got. A failed one keeps the data from before.
    pub fn took(&mut self, got: std::result::Result<T::Data, String>) {
        match got {
            Ok(data) => {
                self.data = Some(data);
                self.news = News::Updated(SystemTime::now());
            }
            Err(e) => self.news = News::Failed(e.lines().next().unwrap_or("").to_string()),
        }
        self.row_in_bounds();
    }

    fn rows(&self) -> usize {
        self.data.as_ref().map(|d| self.tui.rows(d, self.at.screen)).unwrap_or(0)
    }

    fn row_in_bounds(&mut self) {
        self.at.row = self.at.row.min(self.rows().saturating_sub(1));
        if self.rows() == 0 {
            self.at.open = false;
        }
    }

    fn go(&mut self, by: isize) {
        let n = self.tui.screens().len().max(1) as isize;
        self.at = At { screen: (self.at.screen as isize + by).rem_euclid(n) as usize, row: 0, open: false };
    }

    /// A key: what it asks for next.
    pub fn key(&mut self, k: KeyEvent) -> Step {
        if k.kind != KeyEventKind::Press {
            return Step::Stay;
        }
        let last = self.rows().saturating_sub(1);
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => return Step::Quit,
            KeyCode::Char('q') => return Step::Quit,
            KeyCode::Char('?') => self.help = !self.help,
            KeyCode::Esc if self.help => self.help = false,
            KeyCode::Esc => self.at.open = false,
            KeyCode::Char('r') => return Step::Refresh,
            KeyCode::Tab => self.go(1),
            KeyCode::BackTab => self.go(-1),
            KeyCode::Up | KeyCode::Char('k') => self.at.row = self.at.row.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => self.at.row = (self.at.row + 1).min(last),
            KeyCode::PageUp => self.at.row = self.at.row.saturating_sub(10),
            KeyCode::PageDown => self.at.row = (self.at.row + 10).min(last),
            KeyCode::Home | KeyCode::Char('g') => self.at.row = 0,
            KeyCode::End | KeyCode::Char('G') => self.at.row = last,
            KeyCode::Enter => self.at.open = self.rows() > 0,
            _ => {}
        }
        Step::Stay
    }

    /// The whole frame: top bar, tabs, the TUI's screen, the keys, and help over it when it's up.
    pub fn draw(&self, f: &mut Frame) {
        let tabs = self.tui.screens().len() > 1;
        let [top, bar, body, bottom] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(tabs as u16),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(f.area());
        let title =
            Line::from(Span::styled(format!(" {}", self.tui.title()), Style::new().add_modifier(Modifier::BOLD)));
        f.render_widget(Paragraph::new(title), top);
        let news = match &self.news {
            News::Loading => Line::from("loading… ").style(dim()),
            News::Updated(t) => Line::from(format!("updated {} ", ago(*t))).style(dim()),
            News::Failed(e) => Line::from(format!("{e} ")).style(bad()),
        };
        let width = (news.width() as u16).min(top.width.saturating_sub(self.tui.title().len() as u16 + 2));
        let [_, right] = Layout::horizontal([Constraint::Min(0), Constraint::Length(width)]).areas(top);
        f.render_widget(Paragraph::new(news.right_aligned()), right);
        if tabs {
            f.render_widget(
                Tabs::new(self.tui.screens())
                    .select(self.at.screen)
                    .divider(" ")
                    .style(dim())
                    .highlight_style(Style::new().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                bar,
            );
        }
        match &self.data {
            Some(data) => self.tui.draw(f, body, data, &self.at),
            None => {
                let says = match &self.news {
                    News::Failed(e) => Line::from(e.clone()).style(bad()),
                    _ => Line::from("Loading…").style(dim()),
                };
                f.render_widget(Paragraph::new(says).wrap(Wrap { trim: true }).block(boxed("")), body);
            }
        }
        // the longest that fits: a narrow panel (a phone's) keeps `? keys` and `q quit` whole
        let all = [
            if tabs { " tab screens  ↑↓ move  ⏎ open  esc back  r refresh  ? keys  q quit" } else { "" },
            " ↑↓ move  ⏎ open  esc back  r refresh  ? keys  q quit",
            " ↑↓ move  ⏎ open  r refresh  ? keys  q quit",
            " ? keys  q quit",
        ];
        let width = bottom.width as usize;
        let keys = all.iter().find(|k| !k.is_empty() && k.chars().count() <= width).unwrap_or(&all[3]);
        f.render_widget(Paragraph::new(Line::from(*keys).style(dim())), bottom);
        if self.help {
            self.draw_help(f, body);
        }
    }

    fn draw_help(&self, f: &mut Frame, area: Rect) {
        let mut rows: Vec<(String, String)> = vec![];
        if self.tui.screens().len() > 1 {
            rows.push(("tab  ⇧tab".into(), "Next or previous screen".into()));
        }
        for (k, d) in [
            ("↑↓  j k", "Move"),
            ("g  G", "First or last"),
            ("⏎  esc", "Open · back"),
            ("r", "Refresh now"),
            ("?", "These keys"),
            ("q", "Quit"),
        ] {
            rows.push((k.into(), d.into()));
        }
        rows.extend(self.tui.keys());
        let lines: Vec<Line> = rows.iter().map(|(k, d)| Line::from(format!("{k:<12}{d}"))).collect();
        let (w, h) = (44.min(area.width), (lines.len() as u16 + 2).min(area.height));
        let at = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
        f.render_widget(Clear, at);
        f.render_widget(Paragraph::new(lines).block(boxed("Keys")), at);
    }
}

/// Runs the TUI: in a terminal, till `q`; with `--snapshot`, prints one frame as text and returns.
pub fn main<T: Tui>(tui: T) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--snapshot") {
        for line in snapshot_from_args(tui, &args)? {
            println!("{line}");
        }
        return Ok(());
    }
    if !(std::io::stdin().is_terminal() && std::io::stdout().is_terminal()) {
        bail!("{} draws in a terminal: run it in one, or with --snapshot to print a frame", tui.title());
    }
    run(tui)
}

fn snapshot_from_args<T: Tui>(tui: T, args: &[String]) -> Result<Vec<String>> {
    let after = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1));
    let (w, h) = match after("--size") {
        Some(s) => {
            let (w, h) = s.split_once('x').context("--size WxH, like 80x24")?;
            (w.parse().context("--size WxH")?, h.parse().context("--size WxH")?)
        }
        None => (80, 24),
    };
    let number = |flag: &str| after(flag).map(|n| n.parse::<usize>().with_context(|| format!("{flag} N"))).transpose();
    let at = At {
        screen: number("--screen")?.unwrap_or(0),
        row: number("--row")?.unwrap_or(0),
        open: args.iter().any(|a| a == "--open"),
    };
    let got = (tui.loader())().map_err(|e| format!("{e:#}"));
    let mut app = App::new(tui);
    app.took(got);
    app.at = At { screen: at.screen.min(app.tui.screens().len().saturating_sub(1)), ..at };
    app.row_in_bounds();
    Ok(snapshot(&app, w, h))
}

/// One frame of `app` as text, a line per row: what `--snapshot` prints, and what tests read.
pub fn snapshot<T: Tui>(app: &App<T>, width: u16, height: u16) -> Vec<String> {
    let mut term = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    term.draw(|f| app.draw(f)).expect("a drawn frame");
    let buf = term.backend().buffer().clone();
    (0..height)
        .map(|y| (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
        .collect()
}

fn run<T: Tui>(tui: T) -> Result<()> {
    let (got_tx, got_rx) = mpsc::channel();
    let (ask_tx, ask_rx) = mpsc::channel::<()>();
    let (load, every) = (tui.loader(), tui.every());
    std::thread::spawn(move || loop {
        if got_tx.send(load().map_err(|e| format!("{e:#}"))).is_err() {
            return;
        }
        match ask_rx.recv_timeout(every) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => while ask_rx.try_recv().is_ok() {},
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    });
    let mut app = App::new(tui);
    let mut term = ratatui::try_init()?;
    let ran = (|| -> Result<()> {
        loop {
            while let Ok(got) = got_rx.try_recv() {
                app.took(got);
            }
            term.draw(|f| app.draw(f))?;
            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(k) = event::read()? {
                    match app.key(k) {
                        Step::Quit => return Ok(()),
                        Step::Refresh => {
                            app.news = News::Loading;
                            let _ = ask_tx.send(());
                        }
                        Step::Stay => {}
                    }
                }
            }
        }
    })();
    ratatui::restore();
    ran
}

// ------------------------------------------------------------------------------------------- for TUIs to draw with

/// A box with a dim border, titled when `title` isn't empty.
pub fn boxed(title: &str) -> Block<'_> {
    let b = Block::bordered().border_style(dim());
    if title.is_empty() {
        b
    } else {
        b.title(format!(" {title} "))
    }
}

/// A list in a box with the person's row picked, scrolled to keep it in view.
pub fn list(f: &mut Frame, area: Rect, title: &str, items: Vec<ListItem>, at: &At) {
    let mut state = ListState::default().with_selected((!items.is_empty()).then_some(at.row));
    let list = List::new(items).block(boxed(title)).highlight_style(picked());
    f.render_stateful_widget(list, area, &mut state);
}

/// A list beside the open row's detail, or the list alone across the area when nothing is open.
pub fn list_and_detail(area: Rect, at: &At) -> (Rect, Option<Rect>) {
    if !at.open {
        return (area, None);
    }
    if area.width < 100 {
        let [list, detail] = Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);
        return (list, Some(detail));
    }
    let [list, detail] = Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    (list, Some(detail))
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}
pub fn ok() -> Style {
    Style::new().fg(Color::Green)
}
pub fn warn() -> Style {
    Style::new().fg(Color::Yellow)
}
pub fn bad() -> Style {
    Style::new().fg(Color::Red)
}
pub fn picked() -> Style {
    Style::new().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
}

/// How long ago, as people say it: "just now", "12s ago", "4m ago", "2h ago", "3d ago".
pub fn ago(t: SystemTime) -> String {
    ago_secs(SystemTime::now().duration_since(t).map(|d| d.as_secs() as i64).unwrap_or(0))
}

/// [`ago`] for a number of seconds.
pub fn ago_secs(secs: i64) -> String {
    match secs {
        s if s < 5 => "just now".into(),
        s if s < 60 => format!("{s}s ago"),
        s if s < 3600 => format!("{}m ago", s / 60),
        s if s < 86_400 => format!("{}h ago", s / 3600),
        s => format!("{}d ago", s / 86_400),
    }
}

/// Seconds since 1970, now.
pub fn now() -> i64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ------------------------------------------------------------------------------------------- for TUIs to load with

/// A command's output: its stdout, or an error with its first line of stderr. It gives up after `wait`.
pub fn run_out(program: &str, args: &[&str], wait: Duration) -> Result<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("running {program}"))?;
    // read both while it runs: a command whose output fills the pipe (64 KB) blocks until someone reads it
    // each pipe's bytes come back on a channel, so waiting for them has a limit too
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            let _ = tx.send(buf);
        });
        rx
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > wait {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{program} took over {}s", wait.as_secs());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // a child it left running in the background can hold the pipes open after it exits: give up at the limit
    let left = |rx: std::sync::mpsc::Receiver<Vec<u8>>| rx.recv_timeout(wait.saturating_sub(started.elapsed()));
    let (Ok(out), Ok(err)) = (left(out), left(err)) else {
        bail!("{program} left its output open past {}s", wait.as_secs());
    };
    if !status.success() {
        let err = String::from_utf8_lossy(&err);
        bail!("{program}: {}", err.lines().find(|l| !l.trim().is_empty()).unwrap_or("failed"));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// A command's output as lines, empty ones left out.
pub fn run_lines(program: &str, args: &[&str]) -> Result<Vec<String>> {
    Ok(run_out(program, args, Duration::from_secs(20))?
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect())
}

/// A command's output as JSON.
pub fn run_json(program: &str, args: &[&str]) -> Result<serde_json::Value> {
    let out = run_out(program, args, Duration::from_secs(20))?;
    serde_json::from_str(&out).with_context(|| format!("{program}'s output isn't JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two screens: three fruit, and nothing.
    struct Fruit;

    impl Tui for Fruit {
        type Data = Vec<&'static str>;
        fn title(&self) -> String {
            "Fruit".into()
        }
        fn screens(&self) -> Vec<String> {
            vec!["Bowl".into(), "Empty".into()]
        }
        fn loader(&self) -> Box<dyn Fn() -> Result<Self::Data> + Send> {
            Box::new(|| Ok(vec!["apple", "pear", "fig"]))
        }
        fn rows(&self, data: &Self::Data, screen: usize) -> usize {
            if screen == 0 {
                data.len()
            } else {
                0
            }
        }
        fn draw(&self, f: &mut Frame, area: Rect, data: &Self::Data, at: &At) {
            let (l, detail) = list_and_detail(area, at);
            list(f, l, "Bowl", data.iter().map(|d| ListItem::new(*d)).collect(), at);
            if let Some(d) = detail {
                f.render_widget(Paragraph::new(format!("all about {}", data[at.row])).block(boxed("")), d);
            }
        }
        fn keys(&self) -> Vec<(String, String)> {
            vec![("e".into(), "Eat it".into())]
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn loaded() -> App<Fruit> {
        let mut app = App::new(Fruit);
        app.took(Ok(vec!["apple", "pear", "fig"]));
        app
    }

    fn has(lines: &[String], what: &str) -> bool {
        lines.iter().any(|l| l.contains(what))
    }

    #[test]
    fn the_shared_keys() {
        let mut app = loaded();
        assert_eq!(app.key(key(KeyCode::Char('j'))), Step::Stay);
        app.key(key(KeyCode::Down));
        app.key(key(KeyCode::Down));
        assert_eq!(app.at().row, 2, "stops at the last row");
        app.key(key(KeyCode::Char('g')));
        assert_eq!(app.at().row, 0);
        app.key(key(KeyCode::Char('G')));
        assert_eq!(app.at().row, 2);
        app.key(key(KeyCode::Enter));
        assert!(app.at().open);
        app.key(key(KeyCode::Esc));
        assert!(!app.at().open);
        app.key(key(KeyCode::Tab));
        assert_eq!(app.at(), At { screen: 1, row: 0, open: false });
        app.key(key(KeyCode::Enter));
        assert!(!app.at().open, "nothing to open on a screen without a list");
        app.key(key(KeyCode::BackTab));
        app.key(key(KeyCode::BackTab));
        assert_eq!(app.at().screen, 1, "round the ends");
        assert_eq!(app.key(key(KeyCode::Char('r'))), Step::Refresh);
        assert_eq!(app.key(key(KeyCode::Char('q'))), Step::Quit);
        assert_eq!(app.key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)), Step::Quit);
    }

    #[test]
    fn the_frame_at_80_by_24() {
        let mut app = loaded();
        let lines = snapshot(&app, 80, 24);
        assert_eq!(lines.len(), 24);
        assert!(lines[0].starts_with(" Fruit") && lines[0].ends_with("updated just now"), "{}", lines[0]);
        assert!(lines[1].contains("Bowl") && lines[1].contains("Empty"), "tabs: {}", lines[1]);
        assert!(has(&lines, "apple") && has(&lines, "fig"));
        assert!(lines[23].contains("tab screens") && lines[23].contains("q quit"), "{}", lines[23]);
        app.key(key(KeyCode::Down));
        app.key(key(KeyCode::Enter));
        assert!(has(&snapshot(&app, 80, 24), "all about pear"), "the open row's detail, below the list at 80");
        app.key(key(KeyCode::Char('?')));
        let lines = snapshot(&app, 80, 24);
        assert!(has(&lines, "Next or previous screen") && has(&lines, "Eat it"), "help, with the TUI's own keys");
        app.key(key(KeyCode::Esc));
        assert!(!has(&snapshot(&app, 80, 24), "Eat it"), "esc closes help first");
        let narrow = snapshot(&app, 50, 12);
        assert_eq!(narrow[11], " ↑↓ move  ⏎ open  r refresh  ? keys  q quit", "the keys that fit, never cut");
        assert!(app.at().open, "and the row stays open");
    }

    #[test]
    fn a_failed_load_keeps_the_data_and_says_why() {
        let mut app = loaded();
        app.took(Err("core didn't answer\nmore".into()));
        let lines = snapshot(&app, 80, 24);
        assert!(lines[0].ends_with("core didn't answer"), "{}", lines[0]);
        assert!(has(&lines, "apple"));
        let mut fresh = App::new(Fruit);
        assert!(has(&snapshot(&fresh, 80, 24), "Loading…"));
        fresh.took(Err("no git here".into()));
        assert!(snapshot(&fresh, 80, 24).iter().filter(|l| l.contains("no git here")).count() >= 2);
    }

    #[test]
    fn a_snapshot_from_the_command_line() {
        let args = |s: &str| s.split(' ').map(str::to_string).collect::<Vec<_>>();
        let lines = snapshot_from_args(Fruit, &args("--snapshot --size 100x10 --row 2 --open")).unwrap();
        assert_eq!(lines.len(), 10);
        assert!(has(&lines, "all about fig"), "{lines:#?}");
        let lines = snapshot_from_args(Fruit, &args("--snapshot --screen 9")).unwrap();
        assert_eq!(lines.len(), 24, "80x24 by default, the last screen at most");
        assert!(snapshot_from_args(Fruit, &args("--snapshot --size big")).is_err());
    }

    #[test]
    fn commands_and_how_long_ago() {
        assert_eq!(run_lines("sh", &["-c", "echo a; echo; echo b"]).unwrap(), ["a", "b"]);
        assert_eq!(run_json("sh", &["-c", "echo '{\"n\": 1}'"]).unwrap()["n"], 1);
        let e = run_out("sh", &["-c", "echo nope >&2; exit 3"], Duration::from_secs(5)).unwrap_err().to_string();
        assert_eq!(e, "sh: nope");
        assert!(run_out("sleep", &["5"], Duration::from_millis(200)).is_err());
        // more than a pipe holds (64 KB): read while it runs, so it finishes rather than blocking till the limit
        let big = run_out("sh", &["-c", "yes 0123456789abcdef | head -n 20000"], Duration::from_secs(5)).unwrap();
        assert_eq!(big.len(), 20_000 * 17);
        // a background child holding the output open: the limit still holds
        let started = Instant::now();
        let e = run_out("sh", &["-c", "sleep 5 & echo started"], Duration::from_millis(500)).unwrap_err().to_string();
        assert!(e.contains("left its output open"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(3));
        assert_eq!(
            (ago_secs(2), ago_secs(42), ago_secs(300), ago_secs(7200), ago_secs(200_000)),
            ("just now".into(), "42s ago".into(), "5m ago".into(), "2h ago".into(), "2d ago".into())
        );
    }
}
