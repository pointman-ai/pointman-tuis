//! The starter: copy this crate to begin a TUI (README.md, "Adding a TUI"). It shows the git repo it runs in: its
//! latest commits, each opening to what it changed, and its branches. Keep its shape: load everything in `load`
//! (it runs off the screen's thread), keep parsing in plain functions you can test, and draw with the kit.

use std::time::Duration;

use anyhow::Result;
use kit::ratatui::layout::Rect;
use kit::ratatui::text::{Line, Span};
use kit::ratatui::widgets::{ListItem, Paragraph, Wrap};
use kit::ratatui::Frame;
use kit::{At, Tui};

/// One commit, as `git log` gives it.
#[derive(Debug, Clone, PartialEq)]
struct Commit {
    sha: String,
    who: String,
    when: i64,
    subject: String,
    /// "3 files changed, 10 insertions(+)", when git says.
    changed: String,
}

/// What a load reads.
struct Data {
    commits: Vec<Commit>,
    branches: Vec<(String, i64)>,
}

/// `git log --format=%x1e%h%x1f%an%x1f%at%x1f%s --shortstat`: a record per commit, its stat on the line after.
fn commits(out: &str) -> Vec<Commit> {
    out.split('\x1e')
        .filter_map(|rec| {
            let mut lines = rec.lines();
            let f: Vec<&str> = lines.next()?.split('\x1f').collect();
            let changed = lines
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("")
                .to_string();
            Some(Commit {
                sha: f.first()?.to_string(),
                who: f.get(1)?.to_string(),
                when: f.get(2)?.parse().ok()?,
                subject: f.get(3)?.to_string(),
                changed,
            })
        })
        .collect()
}

fn load() -> Result<Data> {
    let log = kit::run_out(
        "git",
        &[
            "log",
            "-n",
            "50",
            "--format=%x1e%h%x1f%an%x1f%at%x1f%s",
            "--shortstat",
        ],
        Duration::from_secs(20),
    )?;
    let branches = kit::run_lines(
        "git",
        &[
            "branch",
            "-a",
            "--sort=-committerdate",
            "--format=%(refname:short)\t%(committerdate:unix)",
        ],
    )?
    .iter()
    .filter_map(|l| {
        l.split_once('\t')
            .map(|(b, t)| (b.to_string(), t.parse().unwrap_or(0)))
    })
    .collect();
    Ok(Data {
        commits: commits(&log),
        branches,
    })
}

struct Starter;

impl Tui for Starter {
    type Data = Data;

    fn title(&self) -> String {
        "This repo".into()
    }

    fn screens(&self) -> Vec<String> {
        vec!["Commits".into(), "Branches".into()]
    }

    fn loader(&self) -> Box<dyn Fn() -> Result<Data> + Send> {
        Box::new(load)
    }

    fn every(&self) -> Duration {
        Duration::from_secs(15)
    }

    fn rows(&self, data: &Data, screen: usize) -> usize {
        [data.commits.len(), data.branches.len()][screen]
    }

    fn draw(&self, f: &mut Frame, area: Rect, data: &Data, at: &At) {
        let now = kit::now();
        match at.screen {
            0 => {
                let (list, detail) = kit::list_and_detail(area, at);
                let items = data
                    .commits
                    .iter()
                    .map(|c| {
                        ListItem::new(Line::from(vec![
                            Span::styled(format!("{} ", c.sha), kit::warn()),
                            Span::styled(
                                format!("{:>8}  ", kit::ago_secs(now - c.when)),
                                kit::dim(),
                            ),
                            Span::raw(c.subject.clone()),
                        ]))
                    })
                    .collect();
                kit::list(f, list, "Commits", items, at);
                if let (Some(d), Some(c)) = (detail, data.commits.get(at.row)) {
                    let lines = vec![
                        Line::from(c.subject.clone()),
                        Line::from(""),
                        Line::from(format!(
                            "{} · {} · {}",
                            c.sha,
                            c.who,
                            kit::ago_secs(now - c.when)
                        ))
                        .style(kit::dim()),
                        Line::from(c.changed.clone()),
                    ];
                    f.render_widget(
                        Paragraph::new(lines)
                            .wrap(Wrap { trim: true })
                            .block(kit::boxed(&c.sha)),
                        d,
                    );
                }
            }
            _ => {
                let items = data
                    .branches
                    .iter()
                    .map(|(b, t)| {
                        ListItem::new(Line::from(vec![
                            Span::raw(b.clone()),
                            Span::styled(format!("  {}", kit::ago_secs(now - t)), kit::dim()),
                        ]))
                    })
                    .collect();
                kit::list(f, area, "Branches, latest first", items, at);
            }
        }
    }
}

fn main() -> Result<()> {
    kit::main(Starter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_log_parses_with_its_stats() {
        let out = "\x1eabc1234\x1fJames\x1f1760000000\x1fFirst one\n\n 2 files changed, 3 insertions(+)\n\x1edef5678\x1fpm-cli\x1f1760000100\x1fNo stat\n";
        let got = commits(out);
        assert_eq!(got.len(), 2);
        assert_eq!(
            got[0],
            Commit {
                sha: "abc1234".into(),
                who: "James".into(),
                when: 1760000000,
                subject: "First one".into(),
                changed: "2 files changed, 3 insertions(+)".into()
            }
        );
        assert_eq!(got[1].changed, "");
    }

    #[test]
    fn it_draws_at_80_by_24() {
        let mut app = kit::App::new(Starter);
        app.took(Ok(Data {
            commits: commits(
                "\x1eabc1234\x1fJames\x1f1760000000\x1fFirst one\n\n 2 files changed\n",
            ),
            branches: vec![("main".into(), 1760000000)],
        }));
        let lines = kit::snapshot(&app, 80, 24);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("abc1234") && l.contains("ago  First one")),
            "{lines:#?}"
        );
    }
}
