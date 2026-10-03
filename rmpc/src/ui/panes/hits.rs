//! rormpc: Hits pane. A table of the ranked chart hits written by the `hits` CLI (`hits ... --json PATH`) with
//! a details panel for the selected row. Rows are chart entries, not directories: a missing song is a dimmed
//! row with nothing to play. Enter / double click play the selected owned song (its queue entry if it is
//! already queued, else appended), `a` appends without playing unless already queued; the queue is never
//! replaced. The ranking itself stays in `hits`: the filter
//! column on the left (h/l moves between it and the table) runs `hits --json` in a background thread on Apply.

use std::{
    path::PathBuf,
    process::Command,
    sync::{Arc, Mutex},
    time::SystemTime,
};

use anyhow::{Context, Result};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
};
use serde::Deserialize;

use super::Pane;
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
        events::AppEvent,
    },
    shared::macros::modal,
    ui::{UiEvent, dirstack::DirState, modals::menu::modal::MenuModal, rormpc_actions},
};

#[derive(Debug, Deserialize)]
struct HitsFile {
    version: u32,
    label: String,
    generated_at: String,
    #[serde(default)]
    args: HitsArgs,
    #[serde(default)]
    rank_note: Option<String>,
    rows: Vec<HitsRow>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct HitsArgs {
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    top: Option<String>,
    #[serde(default)]
    genre: Option<String>,
    #[serde(default)]
    owned: bool,
    #[serde(default)]
    show_hidden: bool,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct HitsRow {
    rank: u32,
    pct: f64,
    cohort: u32,
    artist: String,
    title: String,
    year: i32,
    #[serde(default)]
    years: Vec<i32>,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    plays: u32,
    #[serde(default)]
    mbid: Option<String>,
    /// hidden with `hits hide`; present only when the run used --show-hidden
    #[serde(default)]
    hidden: bool,
}

#[derive(Debug)]
pub struct HitsPane {
    path: PathBuf,
    rows: Vec<HitsRow>,
    label: String,
    generated_at: String,
    rank_note: String,
    error: Option<String>,
    loaded_mtime: Option<SystemTime>,
    state: DirState<TableState>,
    table_area: Rect,
    command: Vec<String>,
    filters: Option<Filters>,
    focus_filters: bool,
    filter_sel: usize,
    /// first filter row shown when the column is taller than the pane
    filter_offset: usize,
    filter_area: Rect,
    job: Arc<Mutex<Job>>,
}

impl HitsPane {
    pub fn new(path: String, command: Vec<String>) -> Self {
        Self {
            command: command.iter().map(|c| expand_home(c)).collect(),
            filters: None,
            focus_filters: false,
            filter_sel: 0,
            filter_offset: 0,
            filter_area: Rect::default(),
            job: Arc::new(Mutex::new(Job::default())),
            path: PathBuf::from(expand_home(&path)),
            rows: Vec::new(),
            label: String::new(),
            generated_at: String::new(),
            rank_note: String::new(),
            error: None,
            loaded_mtime: None,
            state: DirState::default(),
            table_area: Rect::default(),
        }
    }

    /// Re-read the JSON file when it changed. A broken file keeps the last good rows and shows the error.
    fn reload(&mut self) {
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mtime.is_some() && mtime == self.loaded_mtime {
            return;
        }
        self.loaded_mtime = mtime;
        match self.read() {
            Ok(file) => {
                if self.filters.is_none() {
                    self.filters = Some(Filters::from_args(&file.args));
                }
                self.rows = file.rows;
                self.label = file.label;
                self.generated_at = file.generated_at;
                self.rank_note = file.rank_note.unwrap_or_default();
                self.error = None;
                self.state.set_content_and_viewport_len(self.rows.len(), self.state_viewport());
                if !self.rows.is_empty() {
                    let keep = self.state.get_selected().filter(|i| *i < self.rows.len()).unwrap_or(0);
                    self.state.select(Some(keep), 0);
                }
            }
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }

    fn state_viewport(&self) -> usize {
        self.table_area.height.saturating_sub(1).into()
    }

    fn read(&self) -> Result<HitsFile> {
        let text = std::fs::read_to_string(&self.path).with_context(|| {
            format!("no hits file at {} (run `hits ... --json` once)", self.path.display())
        })?;
        let file: HitsFile =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", self.path.display()))?;
        anyhow::ensure!(file.version == 1, "unsupported hits file version {}", file.version);
        Ok(file)
    }

    fn selected(&self) -> Option<&HitsRow> {
        self.state.get_selected().and_then(|i| self.rows.get(i))
    }

    /// Queue the selected owned song, optionally playing it; one already queued is not appended again.
    fn enqueue_selected(&self, play: bool, ctx: &Ctx) {
        let Some(path) = self.selected().and_then(|r| r.file.clone()) else {
            return; // missing songs have nothing to queue
        };
        rormpc_actions::queue_file(ctx, path, play, false);
    }

    /// Run `hits` with the current filters in a background thread; one run at a time, a newer Apply during a
    /// run is queued and replaces any older queued one. Results arrive through the JSON file.
    fn apply(&mut self, ctx: &Ctx) {
        let Some(filters) = &self.filters else { return };
        let args = filters.args(&self.path.to_string_lossy());
        let mut job = self.job.lock().expect("hits job lock");
        if job.running {
            job.queued = Some(args);
            return;
        }
        job.running = true;
        job.error = None;
        drop(job);
        let (job, command, sender) = (Arc::clone(&self.job), self.command.clone(), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let mut args = args;
            loop {
                let result = Command::new(&command[0]).args(&command[1..]).args(&args).output();
                let error = match result {
                    Ok(out) if out.status.success() => None,
                    Ok(out) => Some(
                        String::from_utf8_lossy(&out.stderr).lines().rev().find(|l| !l.trim().is_empty())
                            .unwrap_or("hits failed").to_owned(),
                    ),
                    Err(err) => Some(format!("cannot run {}: {err}", command[0])),
                };
                let mut j = job.lock().expect("hits job lock");
                j.error = error;
                match j.queued.take() {
                    Some(next) => args = next,
                    None => {
                        j.running = false;
                        j.finished = true;
                        break;
                    }
                }
            }
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    fn filter_rows(&self) -> Vec<FilterRow> {
        self.filters.as_ref().map(Filters::rows).unwrap_or_default()
    }

    fn filter_action(&mut self, action: &CommonAction, ctx: &Ctx) -> bool {
        let rows = self.filter_rows();
        let Some(&row) = rows.get(self.filter_sel) else { return false };
        let Some(filters) = self.filters.as_mut() else { return false };
        match action {
            CommonAction::Down => self.filter_sel = snap(&rows, self.filter_sel + 1, true),
            CommonAction::Up => self.filter_sel = snap(&rows, self.filter_sel.saturating_sub(1), false),
            CommonAction::Top => self.filter_sel = snap(&rows, 0, true),
            CommonAction::Bottom => self.filter_sel = snap(&rows, rows.len() - 1, false),
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::Apply => self.apply(ctx),
            CommonAction::Confirm | CommonAction::Select => filters.toggle(row),
            CommonAction::Left => {
                filters.adjust(row, -1);
            }
            CommonAction::Right => {
                if !filters.adjust(row, 1) {
                    self.focus_filters = false;
                }
            }
            _ => return false,
        }
        // the row list changes when switching decades <-> range
        let rows = self.filter_rows();
        self.filter_sel = snap(&rows, self.filter_sel, true);
        self.scroll_filters(0);
        true
    }

    /// Scroll so the cursor stays visible (keyboard) or by `delta` rows (mouse wheel), within bounds.
    fn scroll_filters(&mut self, delta: isize) {
        let height = usize::from(self.filter_area.height).max(1);
        let max = self.filter_rows().len().saturating_sub(height);
        if delta == 0 {
            if self.filter_sel < self.filter_offset {
                self.filter_offset = self.filter_sel;
            } else if self.filter_sel >= self.filter_offset + height {
                self.filter_offset = self.filter_sel + 1 - height;
            }
        } else {
            self.filter_offset = self.filter_offset.saturating_add_signed(delta);
        }
        self.filter_offset = self.filter_offset.min(max);
    }

    fn render_filters(&self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let Some(filters) = &self.filters else { return };
        let rows = filters.rows();
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .skip(self.filter_offset)
            .map(|(i, row)| {
                let text = filters.line(*row);
                let label_style = if matches!(
                    row,
                    FilterRow::Heading(_) | FilterRow::Source | FilterRow::Sort | FilterRow::Mode | FilterRow::Apply
                ) {
                    ctx.config.theme.preview_label_style
                } else {
                    Style::default()
                };
                let cursor = i == self.filter_sel;
                // the cursor is a gutter marker, never a background: colour on "[x]" would read as "checked".
                // Without focus the marker stays dim, so h/l returns to the same row.
                let gutter = match (cursor, self.focus_filters) {
                    (true, true) => Span::styled("›", ctx.config.theme.preview_label_style.add_modifier(Modifier::BOLD)),
                    (true, false) => Span::styled("›", Style::default().add_modifier(Modifier::DIM)),
                    _ => Span::raw(" "),
                };
                let at = text.find(['[', '‹']).unwrap_or(text.len());
                let (label, control) = text.split_at(at);
                let mut control_style = Style::default();
                if ["[x]", "[+]", "[-]"].iter().any(|on| control.starts_with(on)) {
                    control_style = ctx.config.theme.preview_label_style; // checked state lives in the box
                }
                if cursor && self.focus_filters {
                    control_style = control_style.add_modifier(Modifier::BOLD);
                }
                Line::from(vec![
                    gutter,
                    Span::styled(label.to_owned(), label_style),
                    Span::styled(control.to_owned(), control_style),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
    }

    /// Menu for the selected row: play/queue and like for owned songs, hide/unhide for any chart song, and,
    /// kept apart, trashing the owned file. Labels show the key that does the same thing directly.
    fn open_context_menu(&self, ctx: &Ctx) {
        let Some(r) = self.selected().cloned() else { return };
        let job = Arc::clone(&self.job);
        let sender = ctx.app_event_sender.clone();
        let mut menu = MenuModal::new(ctx);
        if let Some(file) = r.file.clone() {
            let (play, queue, copy) = (file.clone(), file.clone(), file.clone());
            menu = menu.list_section(ctx, move |mut section| {
                section.add_item("Play now  (Enter)", move |ctx| {
                    rormpc_actions::queue_file(ctx, play, true, false);
                    Ok(())
                });
                section.add_item("Add to queue  (a)", move |ctx| {
                    rormpc_actions::queue_file(ctx, queue, false, false);
                    Ok(())
                });
                section.add_item("Add another copy", move |ctx| {
                    rormpc_actions::queue_file(ctx, copy, false, true);
                    Ok(())
                });
                Some(section)
            });
            let hint = rormpc_actions::rate_key_hint(ctx);
            let like_file = file.clone();
            menu = menu.list_section(ctx, move |mut section| {
                for (label, value) in [("Like ♥", "2"), ("Dislike ✗", "0"), ("Clear like", "1")] {
                    let file = like_file.clone();
                    section.add_item(format!("{label}{hint}"), move |ctx| {
                        rormpc_actions::set_like(ctx, file, value);
                        Ok(())
                    });
                }
                Some(section)
            });
        }
        let (verb, label) = if r.hidden { ("unhide", "Unhide song (show it in Hits again)") } else { ("hide", "Hide song across charts") };
        let (artist, title, mbid, command) = (r.artist.clone(), r.title.clone(), r.mbid.clone(), self.command.clone());
        menu = menu.list_section(ctx, move |mut section| {
            section.add_item(label, move |_| {
                std::thread::spawn(move || {
                    let mut args = vec![verb.to_owned(), "--artist".to_owned(), artist, "--title".to_owned(), title];
                    if let Some(m) = mbid {
                        args.extend(["--mbid".to_owned(), m]);
                    }
                    let ok = Command::new(&command[0]).args(&command[1..]).args(&args).status().is_ok_and(|s| s.success());
                    let mut j = job.lock().expect("hits job lock");
                    if ok {
                        j.rerun = true;
                    } else {
                        j.error = Some(format!("hits {verb} failed"));
                    }
                    drop(j);
                    let _ = sender.send(AppEvent::RequestRender);
                });
                Ok(())
            });
            Some(section)
        });
        if let Some(file) = r.file.clone() {
            let hint = rormpc_actions::external_key_hint(ctx, &["musicdb", "delete"]);
            menu = menu.list_section(ctx, move |mut section| {
                section.add_item(format!("Delete library file…{hint}"), move |ctx| {
                    rormpc_actions::open_delete_menu(ctx, vec![file]);
                    Ok(())
                });
                Some(section)
            });
        }
        let menu = menu.list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(())))).build();
        modal!(ctx, menu);
    }

    fn details(&self, ctx: &Ctx) -> Vec<Line<'static>> {
        let Some(r) = self.selected() else {
            return vec![Line::from("No hits loaded.")];
        };
        let key = ctx.config.theme.preview_label_style;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let field = |name: &str, value: String| {
            Line::from(vec![Span::styled(format!("{name}: "), key), Span::raw(value)])
        };
        let years = if r.years.is_empty() {
            r.year.to_string()
        } else {
            r.years.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
        };
        let mut lines = vec![
            Line::from(Span::styled(r.title.clone(), Style::default().add_modifier(Modifier::BOLD))),
            Line::from(r.artist.clone()),
            Line::default(),
            field("Rank", format!("#{} of {} ({:.0}%)", r.rank, r.cohort, r.pct.ceil())),
            Line::from(Span::styled(self.rank_note.clone(), dim)),
            field("Year-end charts", years),
            field("Genres", if r.genres.is_empty() { "unknown".to_owned() } else { r.genres.join(", ") }),
            Line::default(),
        ];
        match &r.file {
            Some(file) => {
                lines.push(field("In library", file.clone()));
                lines.push(field("Plays", r.plays.to_string()));
                lines.push(Line::default());
                lines.push(Line::from(Span::styled("Enter: queue and play · a: queue", dim)));
            }
            None => lines.push(field("In library", "no (not found in MPD)".to_owned())),
        }
        if r.hidden {
            lines.push(Line::from(Span::styled("hidden from Hits (menu: Unhide)", dim)));
        }
        lines
    }
}

fn expand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_owned(),
    }
}

impl Pane for HitsPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let rerun = std::mem::take(&mut self.job.lock().expect("hits job lock").rerun);
        if rerun {
            self.apply(ctx);
        }
        let finished = std::mem::take(&mut self.job.lock().expect("hits job lock").finished);
        if finished {
            self.loaded_mtime = None; // hits rewrote the file
            self.reload();
        }
        let [filter_area, main, details] =
            Layout::horizontal([Constraint::Length(24), Constraint::Min(40), Constraint::Percentage(30)])
                .spacing(2)
                .areas(area);
        self.filter_area = filter_area;
        self.scroll_filters(0); // the pane may have been resized
        self.render_filters(frame, filter_area, ctx);
        let [table_area, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(main);
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.rows.len(), self.state_viewport());

        let dim = Style::default().add_modifier(Modifier::DIM);
        let rows = self.rows.iter().map(|r| {
            let owned = r.file.is_some();
            Row::new(vec![
                Cell::from(format!("#{}", r.rank)),
                Cell::from(format!("{:.0}%", r.pct.ceil())),
                Cell::from(if r.hidden { "h" } else if owned { "✓" } else { "✗" }),
                Cell::from(r.artist.clone()),
                Cell::from(r.title.clone()),
                Cell::from(r.year.to_string()),
                Cell::from(if owned && r.plays > 0 { r.plays.to_string() } else { String::new() }),
            ])
            .style(if owned && !r.hidden { Style::default() } else { dim })
        });
        let header = Row::new(["Rank", "%", "", "Artist", "Title", "Year", "Plays"])
            .style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Percentage(35),
            Constraint::Percentage(65),
            Constraint::Length(4),
            Constraint::Length(5),
        ])
        .header(header)
        .column_spacing(1)
        .style(ctx.config.as_text_style())
        .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let owned = self.rows.iter().filter(|r| r.file.is_some()).count();
        let (running, job_error) = {
            let j = self.job.lock().expect("hits job lock");
            (j.running, j.error.clone())
        };
        let status = match (running, job_error.or_else(|| self.error.clone())) {
            (true, _) => Span::styled(" running hits… (the table shows the previous result)", Style::default().add_modifier(Modifier::BOLD)),
            (false, Some(err)) => Span::styled(err, Style::default().add_modifier(Modifier::BOLD)),
            (false, None) => Span::styled(
                format!(
                    " {} · {} hits · {} in library · updated {}",
                    self.label,
                    self.rows.len(),
                    owned,
                    self.generated_at.replace('T', " ")
                ),
                dim,
            ),
        };
        frame.render_widget(Paragraph::new(Line::from(status)), footer);
        frame.render_widget(Paragraph::new(self.details(ctx)).wrap(Wrap { trim: false }), details);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.reload();
        ctx.render()?;
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        if matches!(event, UiEvent::Database | UiEvent::Reconnected) && is_visible {
            self.loaded_mtime = None;
            self.reload();
            ctx.render()?;
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if self.filter_area.contains(event.into()) {
            match event.kind {
                MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                    let idx = self.filter_offset + usize::from(event.y.saturating_sub(self.filter_area.y));
                    if self.filter_rows().get(idx).is_some_and(|row| !matches!(row, FilterRow::Heading(_))) {
                        self.focus_filters = true;
                        self.filter_sel = idx;
                        self.filter_action(&CommonAction::Confirm, ctx);
                    }
                }
                // the wheel scrolls the view; the cursor follows only if it would leave it
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                    let step = ctx.config.scroll_amount.max(1) as isize;
                    self.scroll_filters(if matches!(event.kind, MouseEventKind::ScrollDown) { step } else { -step });
                    let height = usize::from(self.filter_area.height).max(1);
                    let sel = self.filter_sel.clamp(self.filter_offset, self.filter_offset + height - 1);
                    self.filter_sel = snap(&self.filter_rows(), sel, sel > self.filter_sel);
                }
                _ => return Ok(()),
            }
            ctx.render()?;
            return Ok(());
        }
        if !self.table_area.contains(event.into()) {
            return Ok(());
        }
        self.focus_filters = false;
        let row = usize::from(event.y.saturating_sub(self.table_area.y + 1)); // +1: header row
        match event.kind {
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                if let Some(idx) = self.state.get_at_rendered_row(row) {
                    self.state.select(Some(idx), ctx.config.scrolloff);
                    if matches!(event.kind, MouseEventKind::DoubleClick) {
                        self.enqueue_selected(true, ctx);
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                self.state.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff);
            }
            MouseEventKind::ScrollDown => {
                self.state.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff);
            }
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        if self.focus_filters {
            if self.filter_action(&action, ctx) {
                ctx.render()?;
            } else {
                event.abandon();
            }
            return Ok(());
        }
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        match action {
            CommonAction::Left => self.focus_filters = true,
            CommonAction::ContextMenu => self.open_context_menu(ctx),
            CommonAction::Down => self.state.next(scrolloff, wrap),
            CommonAction::Up => self.state.prev(scrolloff, wrap),
            CommonAction::DownHalf => self.state.next_half_viewport(scrolloff),
            CommonAction::UpHalf => self.state.prev_half_viewport(scrolloff),
            CommonAction::PageDown => self.state.next_viewport(scrolloff),
            CommonAction::PageUp => self.state.prev_viewport(scrolloff),
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::Confirm => self.enqueue_selected(true, ctx),
            CommonAction::AddOptions { .. } => self.enqueue_selected(false, ctx), // the "a" key
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        ctx.render()?;
        Ok(())
    }
}

// ---------------------------------------------------------------- filters (rormpc)

const DECADES: [i32; 8] = [1950, 1960, 1970, 1980, 1990, 2000, 2010, 2020];
const TOPS: [(u32, u32); 3] = [(1, 10), (11, 20), (21, 50)];
const GENRES: [&str; 18] = [
    "rock", "pop", "hip hop", "r&b", "soul", "dance", "electronic", "disco", "funk", "country", "metal",
    "folk", "latin", "jazz", "blues", "punk", "reggae", "classical",
];

/// Nearest row at or after (`forward`) / before `i` the cursor may stop on: headings are skipped, and at either
/// end it turns back, so the cursor never rests on a heading.
fn snap(rows: &[FilterRow], i: usize, forward: bool) -> usize {
    let i = i.min(rows.len().saturating_sub(1));
    let stop = |j: &usize| !matches!(rows[*j], FilterRow::Heading(_));
    let after = || (i..rows.len()).find(stop);
    let before = || (0..=i).rev().find(stop);
    if forward { after().or_else(before) } else { before().or_else(after) }.unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FilterRow {
    /// group title on its own line; the cursor skips it
    Heading(&'static str),
    Source,
    Sort,
    Mode,
    Decade(usize),
    From,
    To,
    Top(usize),
    Genre(usize),
    Owned,
    ShowHidden,
    Apply,
}

/// What the filter column edits; turned into `hits` arguments on Apply.
#[derive(Debug, Clone)]
struct Filters {
    /// false: Billboard year-end charts, true: my liked songs
    likes: bool,
    /// likes only: false = by plays, true = rediscover
    rediscover: bool,
    by_range: bool,
    decades: [bool; 8],
    from: i32,
    to: i32,
    tops: [bool; 3],
    /// -1 exclude, 0 off, 1 include
    genres: [i8; GENRES.len()],
    owned: bool,
    show_hidden: bool,
}

impl Default for Filters {
    fn default() -> Self {
        let mut decades = [false; 8];
        decades[3] = true; // 1980s
        Self { likes: false, rediscover: false, by_range: false, decades, from: 1985, to: 1992, tops: [true, false, false], genres: [0; GENRES.len()], owned: false, show_hidden: false }
    }
}

impl Filters {
    fn rows(&self) -> Vec<FilterRow> {
        let mut rows = vec![FilterRow::Source];
        if self.likes {
            rows.push(FilterRow::Sort);
        }
        rows.push(FilterRow::Mode);
        if self.by_range {
            rows.extend([FilterRow::From, FilterRow::To]);
        } else {
            rows.extend((0..DECADES.len()).map(FilterRow::Decade));
        }
        rows.push(FilterRow::Heading("Top %"));
        rows.extend((0..TOPS.len()).map(FilterRow::Top));
        rows.push(FilterRow::Heading("Genres  +in  -out"));
        rows.extend((0..GENRES.len()).map(FilterRow::Genre));
        rows.push(FilterRow::Heading("Options"));
        rows.extend([FilterRow::Owned, FilterRow::ShowHidden, FilterRow::Apply]);
        rows
    }

    fn years(&self) -> String {
        if self.by_range {
            return format!("{}-{}", self.from.min(self.to), self.from.max(self.to));
        }
        let ranges: Vec<String> = DECADES
            .iter()
            .zip(self.decades)
            .filter(|(_, on)| *on)
            .map(|(d, _)| format!("{}-{}", d, d + 9))
            .collect();
        if ranges.is_empty() { "1980-1989".to_owned() } else { ranges.join(",") }
    }

    fn genre_spec(&self) -> String {
        GENRES
            .iter()
            .zip(self.genres)
            .filter(|(_, s)| *s != 0)
            .map(|(g, s)| format!("{}{g}", if s > 0 { '+' } else { '-' }))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn args(&self, json: &str) -> Vec<String> {
        let tops: Vec<String> =
            TOPS.iter().zip(self.tops).filter(|(_, on)| *on).map(|((lo, hi), _)| format!("{lo}-{hi}")).collect();
        let mut args = Vec::new();
        if self.likes {
            args.extend(["--source".to_owned(), "likes".to_owned(), "--sort".to_owned()]);
            args.push(if self.rediscover { "rediscover" } else { "plays" }.to_owned());
        }
        // likes without any decade ticked = all years (a chart needs a period)
        if !(self.likes && !self.by_range && !self.decades.iter().any(|d| *d)) {
            args.extend(["--years".to_owned(), self.years()]);
        }
        args.push("--top".to_owned());
        args.push(if tops.is_empty() { "1-100".to_owned() } else { tops.join(",") });
        let genres = self.genre_spec();
        if !genres.is_empty() {
            args.extend(["-g".to_owned(), genres]);
        }
        if self.owned {
            args.push("--owned".to_owned());
        }
        if self.show_hidden {
            args.push("--show-hidden".to_owned());
        }
        args.extend(["--json".to_owned(), json.to_owned()]);
        args
    }

    /// Start from what produced the current file, so the column matches the table.
    fn from_args(args: &HitsArgs) -> Self {
        let mut f = Self::default();
        let period = args.period.clone().unwrap_or_default();
        let parts: Vec<(i32, i32)> = period
            .split(',')
            .filter_map(|p| {
                let (lo, hi) = p.trim().split_once('-').unwrap_or((p.trim(), p.trim()));
                Some((lo.trim_end_matches('s').parse().ok()?, hi.parse().unwrap_or(-1)))
            })
            .collect();
        let decade_parts: Vec<usize> = parts
            .iter()
            .filter_map(|(lo, hi)| {
                let is_decade = lo % 10 == 0 && (*hi == lo + 9 || *hi == -1);
                is_decade.then(|| DECADES.iter().position(|d| d == lo)).flatten()
            })
            .collect();
        if !parts.is_empty() && decade_parts.len() == parts.len() {
            f.decades = [false; 8];
            decade_parts.into_iter().for_each(|i| f.decades[i] = true);
        } else if let Some((lo, hi)) = parts.first() {
            f.by_range = true;
            f.from = *lo;
            f.to = if *hi > 0 { *hi } else { *lo };
        }
        if let Some(top) = &args.top {
            f.tops = [false; 3];
            for part in top.split(',') {
                if let Some(i) = TOPS.iter().position(|(lo, hi)| part.trim() == format!("{lo}-{hi}")) {
                    f.tops[i] = true;
                }
            }
        }
        for tok in args.genre.clone().unwrap_or_default().split_whitespace() {
            let (sign, name) = match tok.chars().next() {
                Some('-') => (-1, &tok[1..]),
                Some('+') => (1, &tok[1..]),
                _ => (1, tok),
            };
            if let Some(i) = GENRES.iter().position(|g| *g == name) {
                f.genres[i] = sign;
            }
        }
        f.owned = args.owned;
        f.show_hidden = args.show_hidden;
        f.likes = args.source.as_deref() == Some("likes");
        f.rediscover = args.sort.as_deref() == Some("rediscover");
        if f.likes && args.period.is_none() {
            f.decades = [false; 8]; // all years
        }
        f
    }

    fn line(&self, row: FilterRow) -> String {
        let check = |on: bool| if on { "[x]" } else { "[ ]" };
        match row {
            FilterRow::Source => format!("Source: {}", if self.likes { "‹my likes›" } else { "‹Billboard US›" }),
            FilterRow::Sort => format!("Sort:   {}", if self.rediscover { "‹rediscover›" } else { "‹by plays›" }),
            FilterRow::Mode => format!("Period: {}", if self.by_range { "‹year range›" } else { "‹decades›" }),
            FilterRow::Decade(i) => format!("  {} {}s", check(self.decades[i]), DECADES[i]),
            FilterRow::From => format!("  from ‹ {} ›", self.from),
            FilterRow::To => format!("  to   ‹ {} ›", self.to),
            // every box sits at the same 2-cell indent under its heading: a hanging label per group made the
            // columns step like an expandable tree
            FilterRow::Heading(title) => title.to_owned(),
            FilterRow::Top(i) => format!("  {} {}-{}%", check(self.tops[i]), TOPS[i].0, TOPS[i].1),
            FilterRow::Genre(i) => {
                let mark = match self.genres[i] { 1 => "+", -1 => "-", _ => " " };
                format!("  [{mark}] {}", GENRES[i])
            }
            FilterRow::Owned => format!("  {} owned only", check(self.owned)),
            FilterRow::ShowHidden => format!("  {} show hidden", check(self.show_hidden)),
            FilterRow::Apply => "  [ Apply ]".to_owned(),
        }
    }

    /// Space / Enter on a row.
    fn toggle(&mut self, row: FilterRow) {
        match row {
            FilterRow::Source => self.likes = !self.likes,
            FilterRow::Sort => self.rediscover = !self.rediscover,
            FilterRow::Mode => self.by_range = !self.by_range,
            FilterRow::Decade(i) => self.decades[i] = !self.decades[i],
            FilterRow::Top(i) => self.tops[i] = !self.tops[i],
            FilterRow::Genre(i) => self.genres[i] = match self.genres[i] { 0 => 1, 1 => -1, _ => 0 },
            FilterRow::Owned => self.owned = !self.owned,
            FilterRow::ShowHidden => self.show_hidden = !self.show_hidden,
            FilterRow::Heading(_) | FilterRow::From | FilterRow::To | FilterRow::Apply => {}
        }
    }

    /// h / l on a year row; false when the row has nothing to adjust.
    fn adjust(&mut self, row: FilterRow, delta: i32) -> bool {
        let clamp = |y: i32| y.clamp(1959, 2025);
        match row {
            FilterRow::From => self.from = clamp(self.from + delta),
            FilterRow::To => self.to = clamp(self.to + delta),
            FilterRow::Mode => self.by_range = !self.by_range,
            FilterRow::Source => self.likes = !self.likes,
            FilterRow::Sort => self.rediscover = !self.rediscover,
            _ => return false,
        }
        true
    }
}

/// State shared with the thread that runs `hits`.
#[derive(Debug, Default)]
struct Job {
    running: bool,
    /// Apply pressed while running: run again with these args when the current run ends
    queued: Option<Vec<String>>,
    error: Option<String>,
    finished: bool,
    /// a hide/unhide changed the result: run hits again with the current filters
    rerun: bool,
}
