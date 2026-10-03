//! rormpc: Hits pane. A table of the ranked chart hits written by the `hits` CLI (`hits ... --json PATH`) with
//! a details panel for the selected row. Rows are chart entries, not directories: a missing song is a dimmed
//! row with nothing to play. Enter / double click append the selected owned song to the queue and play it,
//! `a` appends without playing; the queue is never replaced. The ranking itself stays in `hits`.

use std::{path::PathBuf, time::SystemTime};

use anyhow::{Context, Result};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
};
use rmpc_mpd::client::Client;
use serde::Deserialize;

use super::Pane;
use crate::{
    config::keys::{
        CommonAction,
        actions::{AutoplayKind, Position},
    },
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
        mpd_client_ext::{Enqueue, MpdClientExt},
    },
    ui::{UiEvent, dirstack::DirState},
};

#[derive(Debug, Deserialize)]
struct HitsFile {
    version: u32,
    label: String,
    generated_at: String,
    rows: Vec<HitsRow>,
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
}

#[derive(Debug)]
pub struct HitsPane {
    path: PathBuf,
    rows: Vec<HitsRow>,
    label: String,
    generated_at: String,
    error: Option<String>,
    loaded_mtime: Option<SystemTime>,
    state: DirState<TableState>,
    table_area: Rect,
}

impl HitsPane {
    pub fn new(path: String) -> Self {
        Self {
            path: PathBuf::from(expand_home(&path)),
            rows: Vec::new(),
            label: String::new(),
            generated_at: String::new(),
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
                self.rows = file.rows;
                self.label = file.label;
                self.generated_at = file.generated_at;
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

    /// Append the selected owned song to the end of the queue, optionally playing it.
    fn enqueue_selected(&self, play: bool, ctx: &Ctx) {
        let Some(path) = self.selected().and_then(|r| r.file.clone()) else {
            return; // missing songs have nothing to queue
        };
        Client::resolve_and_enqueue(
            ctx,
            vec![Enqueue::File { path }],
            Position::EndOfQueue,
            if play { AutoplayKind::First } else { AutoplayKind::None },
            ctx.current_song_index(),
            None,
        );
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
            Line::from(Span::styled("chart rank within the chosen years and genres", dim)),
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
        let [main, details] =
            Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)]).areas(area);
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
                Cell::from(if owned { "✓" } else { "✗" }),
                Cell::from(r.artist.clone()),
                Cell::from(r.title.clone()),
                Cell::from(r.year.to_string()),
                Cell::from(if owned && r.plays > 0 { r.plays.to_string() } else { String::new() }),
            ])
            .style(if owned { Style::default() } else { dim })
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
        let status = match &self.error {
            Some(err) => Span::styled(err.clone(), Style::default().add_modifier(Modifier::BOLD)),
            None => Span::styled(
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
        if !self.table_area.contains(event.into()) {
            return Ok(());
        }
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
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        match action {
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
