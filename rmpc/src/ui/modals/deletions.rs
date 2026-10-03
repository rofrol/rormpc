//! rormpc: the deletion queue (songs trashed with Ctrl-x) and finishing them: per song, remove the video from
//! the YouTube playlists (default on) and/or delete its ListenBrainz listens (default off, irreversible).
//! `musicdb deletions` does the work in a background thread; this modal only lists, toggles and confirms.

use std::{
    collections::HashMap,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};
use serde::Deserialize;

use super::{
    Modal,
    confirm_modal::{Action, ConfirmModal},
};
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        events::AppEvent,
        id::{self, Id},
        keys::ActionEvent,
        macros::modal,
        mouse_event::{MouseEvent, MouseEventKind},
    },
};

const MUSICDB: &str = "musicdb";

#[derive(Debug, Clone, Deserialize)]
struct Queued {
    id: String,
    artist: String,
    title: String,
    queued_at: String,
    #[serde(default)]
    ytid: Option<String>,
    #[serde(default)]
    lb_listens: u32,
    #[serde(default)]
    shared: bool,
    #[serde(default)]
    ops: HashMap<String, String>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct Row {
    q: Queued,
    youtube: bool,
    listenbrainz: bool,
}

#[derive(Debug, Default)]
struct Job {
    loading: bool,
    running: bool,
    fresh: Option<Vec<Queued>>,
    message: Option<String>,
}

#[derive(Debug)]
pub struct DeletionsModal {
    id: Id,
    rows: Vec<Row>,
    sel: usize,
    /// 0 = YouTube column, 1 = ListenBrainz column
    col: usize,
    job: Arc<Mutex<Job>>,
    list_area: ratatui::layout::Rect,
}

fn run(args: &[String]) -> Result<String, String> {
    match Command::new(MUSICDB).args(args).output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("musicdb failed")
            .to_owned()),
        Err(err) => Err(format!("cannot run {MUSICDB}: {err}")),
    }
}

/// Fetch the queue into `job.fresh` (picked up by the next render).
fn load(job: &Arc<Mutex<Job>>, sender: crossbeam::channel::Sender<AppEvent>) {
    job.lock().expect("deletions job lock").loading = true;
    let job = Arc::clone(job);
    std::thread::spawn(move || {
        let result = run(&["deletions".into(), "--json".into()])
            .and_then(|out| serde_json::from_str::<Vec<Queued>>(&out).map_err(|e| e.to_string()));
        let mut j = job.lock().expect("deletions job lock");
        j.loading = false;
        match result {
            Ok(rows) => j.fresh = Some(rows),
            Err(err) => j.message = Some(err),
        }
        drop(j);
        let _ = sender.send(AppEvent::RequestRender);
    });
}

impl DeletionsModal {
    pub fn new(ctx: &Ctx) -> Self {
        let modal = Self {
            id: id::new(),
            rows: Vec::new(),
            sel: 0,
            col: 0,
            job: Arc::new(Mutex::new(Job::default())),
            list_area: ratatui::layout::Rect::default(),
        };
        load(&modal.job, ctx.app_event_sender.clone());
        modal
    }

    /// Ask, then finish every listed song with its own choices, one `musicdb` call per song.
    fn confirm(&self, ctx: &Ctx) {
        if self.rows.is_empty() || self.job.lock().expect("deletions job lock").running {
            return;
        }
        let mut message = vec!["Finish these deletions?".to_owned(), String::new()];
        for r in &self.rows {
            let yt = if r.youtube && r.q.ytid.is_some() { "remove from YouTube playlists" } else { "keep YouTube" };
            let lb = if r.listenbrainz && r.q.lb_listens > 0 {
                format!("DELETE {} ListenBrainz listens (irreversible)", r.q.lb_listens)
            } else {
                "keep ListenBrainz".to_owned()
            };
            message.push(format!("• {} - {}: {yt}, {lb}", r.q.artist, r.q.title));
        }
        let plan: Vec<Vec<String>> = self
            .rows
            .iter()
            .map(|r| {
                vec![
                    "deletions".into(),
                    "--confirm".into(),
                    r.q.id.clone(),
                    if r.youtube { "--youtube" } else { "--no-youtube" }.into(),
                    if r.listenbrainz { "--listenbrainz" } else { "--no-listenbrainz" }.into(),
                    "--json".into(),
                ]
            })
            .collect();
        let (job, sender) = (Arc::clone(&self.job), ctx.app_event_sender.clone());
        let on_delete = move |_: &Ctx| -> Result<()> {
            job.lock().expect("deletions job lock").running = true;
            std::thread::spawn(move || {
                let mut failures = Vec::new();
                for args in &plan {
                    if let Err(err) = run(args) {
                        failures.push(err);
                    }
                }
                {
                    let mut j = job.lock().expect("deletions job lock");
                    j.running = false;
                    j.message = Some(if failures.is_empty() {
                        "done".to_owned()
                    } else {
                        format!("{} failed: {}", failures.len(), failures.join("; "))
                    });
                }
                load(&job, sender);
            });
            Ok(())
        };
        // "Cancel" is the first button, so a stray Enter cancels
        modal!(
            ctx,
            ConfirmModal::builder()
                .ctx(ctx)
                .message(message)
                .action(Action::CustomButtons {
                    buttons: vec![
                        ("Cancel", Box::new(|_: &Ctx| Ok(()))),
                        ("Finish deletions", Box::new(on_delete)),
                    ],
                })
                .build()
        );
    }

    fn take_fresh(&mut self) {
        let Some(fresh) = self.job.lock().expect("deletions job lock").fresh.take() else { return };
        let old: HashMap<String, (bool, bool)> =
            self.rows.iter().map(|r| (r.q.id.clone(), (r.youtube, r.listenbrainz))).collect();
        self.rows = fresh
            .into_iter()
            .map(|q| {
                let (youtube, listenbrainz) = old.get(&q.id).copied().unwrap_or((q.ytid.is_some(), false));
                Row { q, youtube, listenbrainz }
            })
            .collect();
        self.sel = self.sel.min(self.rows.len().saturating_sub(1));
    }
}

impl Modal for DeletionsModal {
    fn id(&self) -> Id {
        self.id
    }

    fn render(&mut self, frame: &mut Frame, ctx: &mut Ctx) -> Result<()> {
        self.take_fresh();
        let area = frame.area().centered(Constraint::Percentage(85), Constraint::Percentage(70));
        frame.render_widget(Clear, area);
        if let Some(bg) = ctx.config.theme.modal_background_color {
            frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(ctx.config.as_border_style())
            .title(" Deletion queue ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [list, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(inner);
        self.list_area = list;

        let (loading, running, message) = {
            let j = self.job.lock().expect("deletions job lock");
            (j.loading, j.running, j.message.clone())
        };
        let label = ctx.config.theme.preview_label_style;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let mut lines = vec![Line::from(Span::styled(" YT   LB    Song · deleted · state", label))];
        if self.rows.is_empty() {
            lines.push(Line::from(if loading { " loading…" } else { " nothing queued: Ctrl-x trashes a song, Ctrl-y undoes" }));
        }
        for (i, r) in self.rows.iter().enumerate() {
            let cell = |on: bool, enabled: bool, col: usize| {
                let text = if !enabled { "[-]" } else if on { "[x]" } else { "[ ]" };
                let mut style = if on && enabled { label } else { Style::default() };
                if i == self.sel && self.col == col {
                    style = style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                }
                Span::styled(text, style)
            };
            let state = match (&r.q.error, r.q.ops.is_empty()) {
                (Some(err), _) => format!("error: {err}"),
                (None, false) => r.q.ops.iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", "),
                (None, true) => format!(
                    "{} LB listens{}",
                    r.q.lb_listens,
                    if r.q.shared { " (another copy keeps them)" } else { "" }
                ),
            };
            lines.push(Line::from(vec![
                Span::raw(if i == self.sel { "›" } else { " " }),
                cell(r.youtube, r.q.ytid.is_some(), 0),
                Span::raw("  "),
                cell(r.listenbrainz, r.q.lb_listens > 0 && !r.q.shared, 1),
                Span::raw("  "),
                Span::raw(format!("{} - {}", r.q.artist, r.q.title)),
                Span::styled(format!("  · {} · {state}", r.q.queued_at.replace('T', " ")), dim),
            ]));
        }
        frame.render_widget(Paragraph::new(lines), list);

        let status = if running {
            "running musicdb… (keep this open or close it, the work continues)".to_owned()
        } else {
            message.unwrap_or_default()
        };
        let help = "j/k move · h/l YouTube/ListenBrainz · Space toggle · Enter finish… · Esc close · Ctrl-y undoes the last Ctrl-x";
        frame.render_widget(
            Paragraph::new(vec![Line::from(Span::styled(status, label)), Line::from(Span::styled(help, dim))])
                .wrap(Wrap { trim: true }),
            footer,
        );
        Ok(())
    }

    fn handle_key(&mut self, key: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = key.claim_common().cloned() else { return Ok(()) };
        match action {
            CommonAction::Down => self.sel = (self.sel + 1).min(self.rows.len().saturating_sub(1)),
            CommonAction::Up => self.sel = self.sel.saturating_sub(1),
            CommonAction::Left => self.col = 0,
            CommonAction::Right => self.col = 1,
            CommonAction::Select => {
                if let Some(r) = self.rows.get_mut(self.sel) {
                    if self.col == 0 && r.q.ytid.is_some() {
                        r.youtube = !r.youtube;
                    } else if self.col == 1 && r.q.lb_listens > 0 && !r.q.shared {
                        r.listenbrainz = !r.listenbrainz;
                    }
                }
            }
            CommonAction::Confirm => self.confirm(ctx),
            CommonAction::Close => {
                self.hide(ctx)?;
                return Ok(());
            }
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &mut Ctx) -> Result<()> {
        if matches!(event.kind, MouseEventKind::LeftClick) && self.list_area.contains(event.into()) {
            let idx = usize::from(event.y.saturating_sub(self.list_area.y + 1)); // +1: header line
            if idx < self.rows.len() {
                self.sel = idx;
                ctx.render()?;
            }
        }
        Ok(())
    }
}
