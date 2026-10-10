//! rormpc: the Play pane (plans/combined-view.md, phase 3): Queue, Hits and Shuffle in one tab.
//!
//! - Normal mode (weighted off): MPD's queue in its order; the Hits filter column is collapsed to one line naming
//!   the source, `h` or a click opens it.
//! - Weighted mode (`w`): the plan projection (past plays, `0 ▶`, Up next, the forecast, then the unplanned
//!   rest) with the filter column open. Turning weighted off keeps the queue as it is.
//! - A filter change prepares a preview (`hits` into its own file, MPD untouched): the table shows it under a
//!   banner with its counts; `a` (or Apply) plays it, Esc drops it and shows the playing source again. See
//!   `rormpc_play` for Apply's confirmation and race rules.
//! - The header line always says what plays and how: `▶ Playing from: … · weighted · round 12/84`.
//!
//! It is composed, not copied: the filter column and the preview table are a `HitsPane` in Play mode, the table
//! is a `QueuePane` whose view follows the weighted shuffle. `Body` leaves room for the Browse and Live bodies of
//! phase 3b; only the Queue body exists now.

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{
    Pane,
    hits::{HitsBody, HitsPane},
    queue::QueuePane,
};
use crate::{
    config::keys::{CommonAction, QueueActions},
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        macros::{status_error, status_info},
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        UiEvent,
        input::InputResultEvent,
        rormpc_exceptions::{self, Kind},
        rormpc_play,
        rormpc_player,
        rormpc_upnext::{self, HitsSource},
    },
};

/// Width of the filter column, as in the Hits pane.
const COLUMN_WIDTH: u16 = 24;
const APPLY_BUTTON: &str = "[ a Apply ]";

/// What fills Play under its header line. Phase 3b adds Browse and Live (full-width bodies).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Body {
    Queue,
}

#[derive(Debug)]
pub struct PlayPane {
    hits: HitsPane,
    queue: QueuePane,
    body: Body,
    /// the rules the source being played was applied with (source.json), None for any other source; Esc brings
    /// the filters back to them (or to the defaults)
    baseline: Option<serde_json::Value>,
    /// the hash those rules give the filters: filters with another hash on screen are a preview
    baseline_hash: String,
    /// source.json's rules hash when `baseline` was taken (an Apply anywhere moves it); `synced` once taken
    source_hash: Option<String>,
    synced: bool,
    /// areas of the last render, for the mouse
    collapsed_area: Rect,
    apply_area: Rect,
    queue_area: Rect,
}

impl PlayPane {
    pub fn new(ctx: &Ctx) -> Self {
        let mut s = Self {
            hits: HitsPane::for_play(rormpc_play::PREVIEW_FILE.to_owned(), vec!["hits".to_owned()]),
            queue: QueuePane::new(ctx),
            body: Body::Queue,
            baseline: None,
            baseline_hash: String::new(),
            source_hash: None,
            synced: false,
            collapsed_area: Rect::default(),
            apply_area: Rect::default(),
            queue_area: Rect::default(),
        };
        s.sync(ctx);
        s
    }

    /// Follow what changed outside: the weighted shuffle switches the table's view, and a new source (Apply
    /// here, Sources…, another rormpc) becomes the baseline. An open preview stays open over a new source.
    fn sync(&mut self, ctx: &Ctx) {
        self.queue.follow_weighted(rormpc_player::shuffle_state().enabled, ctx);
        let rules = rormpc_upnext::source_rules();
        let hash = rules.as_ref().map(|(_, h)| h.clone());
        if self.synced && self.source_hash == hash {
            return;
        }
        let previewing = self.synced && self.preview_active();
        self.synced = true;
        self.source_hash = hash;
        self.baseline = rules.map(|(r, _)| r);
        self.baseline_hash = HitsPane::rules_hash_of(self.baseline.as_ref());
        if !previewing {
            self.hits.reset_filters(self.baseline.as_ref());
        }
    }

    fn weighted() -> bool {
        rormpc_player::shuffle_state().enabled
    }

    /// The filters on screen differ from the rules being played.
    fn preview_active(&self) -> bool {
        self.hits.rules_hash().is_some_and(|h| h != self.baseline_hash)
    }

    /// The filter column is shown: always while weighted, else while it has the keys or shows a preview.
    fn column_open(&self) -> bool {
        Self::weighted() || self.hits.focus_filters() || self.preview_active() || self.hits.in_downloads()
    }

    /// The table right of the column is the Hits result (a preview, or the Downloads view), not the queue.
    fn shows_hits_table(&self) -> bool {
        self.preview_active() || self.hits.in_downloads()
    }

    /// Keys go to the Hits part (the column, or its table) rather than the queue.
    fn keys_to_hits(&self) -> bool {
        (self.column_open() && self.hits.focus_filters()) || self.shows_hits_table()
    }

    /// `a`: play the preview. With the filters equal to the source's, Apply plays them again (keeping the round)
    /// once their result is on screen.
    fn apply(&mut self, ctx: &Ctx) {
        let info = self.hits.preview_info();
        if info.running || (!info.ready && info.error.is_none()) {
            if self.preview_active() || info.running {
                return status_info!("The preview is still running: Apply when its counts are in the banner");
            }
            return status_info!("The queue already plays these filters: change one to prepare another source");
        }
        if let Some(err) = info.error {
            return status_error!("No preview to apply: {err}");
        }
        if info.files.is_empty() {
            return status_info!("0 owned songs: widen the filter (the queue stays as it is)");
        }
        let (Some(rules), Some(rules_hash)) = (self.hits.applied_rules(), self.hits.rules_hash()) else {
            return status_info!("The preview is still running: Apply when its counts are in the banner");
        };
        rormpc_play::apply(ctx, HitsSource { name: info.label, files: info.files, rules, rules_hash });
    }

    /// Esc with nothing left to close inside the table: drop the preview, else close the column (normal mode).
    /// False when Play has nothing to do with it.
    fn escape(&mut self) -> bool {
        if self.preview_active() {
            self.hits.reset_filters(self.baseline.as_ref());
            status_info!("Preview dropped: the table shows the playing source again");
            return true;
        }
        if self.hits.focus_filters() && !Self::weighted() {
            self.hits.set_focus_filters(false);
            return true;
        }
        false
    }

    fn header_line(&self, ctx: &Ctx) -> String {
        let source = rormpc_upnext::header(ctx).map_or_else(|| "Nothing chosen yet".to_owned(), |h| h.trim().to_owned());
        let shuffle = rormpc_player::shuffle_state();
        let mode = if shuffle.enabled {
            let round = shuffle.round.as_ref().map_or(String::new(), |r| {
                if r.done { " · round done".to_owned() } else { format!(" · round {}/{}", r.heard.len(), r.total) }
            });
            let waiting = if shuffle.active { String::new() } else { format!(" · waiting: {}", shuffle.reason) };
            format!("weighted{round}{waiting}")
        } else {
            "in queue order · w: weighted".to_owned()
        };
        format!("▶ {source} · {mode}")
    }

    /// The collapsed filter column: the rules of the source in one line.
    fn collapsed_line(&self) -> String {
        let rules = match rormpc_upnext::source_info().map(|(kind, name, _)| (kind, name)) {
            Some((kind, _)) if kind == "hits" && self.baseline.is_some() => self.hits.formula().unwrap_or_default(),
            Some((kind, name)) if kind == "hits" => format!("Hits · {name}"),
            Some((kind, _)) if kind == "library" => "whole library".to_owned(),
            Some((_, name)) => name,
            None => "none yet".to_owned(),
        };
        format!(" Source: {rules}  [h: filters]")
    }

    fn render_queue_body(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let preview = self.preview_active();
        let [banner, rest] =
            Layout::vertical([Constraint::Length(u16::from(preview)), Constraint::Min(1)]).areas(area);
        self.apply_area = Rect::default();
        if preview {
            let (text, enabled) = rormpc_play::banner(&self.hits.preview_info(), ctx.current_song().map(|s| s.file.as_str()));
            let width = APPLY_BUTTON.chars().count() as u16;
            let [t, b] = Layout::horizontal([Constraint::Min(1), Constraint::Length(width)]).spacing(1).areas(banner);
            frame.render_widget(Paragraph::new(Line::from(Span::styled(text, ctx.config.theme.preview_label_style))), t);
            let button = if enabled {
                self.apply_area = b;
                ctx.config.theme.preview_label_style.add_modifier(Modifier::REVERSED | Modifier::BOLD)
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            frame.render_widget(Paragraph::new(Line::from(Span::styled(APPLY_BUTTON, button))), b);
        }
        let (column, table) = if self.column_open() {
            let [c, t] = Layout::horizontal([Constraint::Length(COLUMN_WIDTH), Constraint::Min(1)]).spacing(2).areas(rest);
            self.collapsed_area = Rect::default();
            (Some(c), t)
        } else {
            let [line, t] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(rest);
            self.collapsed_area = line;
            let dim = Style::default().add_modifier(Modifier::DIM);
            frame.render_widget(Paragraph::new(Line::from(Span::styled(self.collapsed_line(), dim))), line);
            (None, t)
        };
        if self.shows_hits_table() {
            let [dl_main, dl_details] =
                Layout::horizontal([Constraint::Min(40), Constraint::Percentage(40)]).spacing(2).areas(table);
            let body = HitsBody { main: table, details: None, dl_main, dl_details };
            self.hits.render_parts(frame, column, Some(body), ctx);
            self.queue_area = Rect::default();
        } else {
            self.hits.render_parts(frame, column, None, ctx);
            self.queue_area = table;
            self.queue.render(frame, table, ctx)?;
        }
        Ok(())
    }
}

impl Pane for PlayPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        self.hits.prepare(ctx);
        if self.hits.take_commit() {
            self.apply(ctx);
        }
        self.sync(ctx);
        let [header, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let line = Line::from(Span::styled(self.header_line(ctx), ctx.config.theme.preview_label_style.add_modifier(Modifier::BOLD)));
        frame.render_widget(Paragraph::new(line), header);
        match self.body {
            Body::Queue => self.render_queue_body(frame, body, ctx),
        }
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.sync(ctx);
        self.hits.before_show(ctx)?;
        self.queue.before_show(ctx)
    }

    fn on_hide(&mut self, ctx: &Ctx) -> Result<()> {
        self.hits.on_hide(ctx)?;
        self.queue.on_hide(ctx)
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        self.hits.on_event(event, is_visible, ctx)?;
        self.queue.on_event(event, is_visible, ctx)
    }

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &mut Ctx) -> Result<()> {
        if self.keys_to_hits() || self.hits.typing() {
            self.hits.handle_insert_mode(kind, ctx)
        } else {
            self.queue.handle_insert_mode(kind, ctx)
        }
    }

    fn handle_insert_nav(&mut self, down: bool, handled: &mut bool, ctx: &mut Ctx) -> Result<()> {
        if self.keys_to_hits() || self.hits.typing() {
            self.hits.handle_insert_nav(down, handled, ctx)
        } else {
            self.queue.handle_insert_nav(down, handled, ctx)
        }
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        self.sync(ctx);
        let common = event.actions.iter().find_map(|a| a.as_common()).cloned();
        let queue_action = event.actions.iter().find_map(|a| a.as_queue()).cloned();
        // `a` is Apply everywhere in Play (the row menu keeps "Add to queue")
        if matches!(common, Some(CommonAction::AddOptions { .. })) && event.claim_common().is_some() {
            self.apply(ctx);
            return Ok(ctx.render()?);
        }
        if self.keys_to_hits() {
            self.hits.handle_action(event, ctx)?;
        } else {
            match (&common, &queue_action) {
                // h: the filter column (it opens in normal mode)
                (Some(CommonAction::Left), _) if event.claim_common().is_some() => {
                    self.hits.set_focus_filters(true);
                    return Ok(ctx.render()?);
                }
                (Some(CommonAction::Close), _) if !self.queue.esc_pending(ctx) && self.escape() => {
                    let _ = event.claim_common();
                    return Ok(ctx.render()?);
                }
                // + / -: the scopes offered are the + sets of the rules being played
                (_, Some(QueueActions::PinSong | QueueActions::ExcludeSong)) if event.claim_queue().is_some() => {
                    let kind = if matches!(queue_action, Some(QueueActions::PinSong)) { Kind::Pin } else { Kind::Exclude };
                    if let Some(song) = self.queue.selected_song(ctx) {
                        let plus = self.baseline.as_ref().map_or([0; 4], rormpc_exceptions::plus_sets_of_args);
                        rormpc_exceptions::open_for_song_with_sets(ctx, kind, &song, plus);
                    } else {
                        status_error!("No song selected");
                    }
                    return Ok(());
                }
                _ => self.queue.handle_action(event, ctx)?,
            }
        }
        if self.hits.take_commit() {
            self.apply(ctx);
        }
        // Esc the Hits part left alone: drop the preview, or close the column
        match event.claim_common() {
            Some(CommonAction::Close) if self.escape() => ctx.render()?,
            Some(_) => event.abandon(),
            None => {}
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        let click = matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::DoubleClick);
        if self.apply_area.contains(event.into()) {
            if click {
                self.apply(ctx);
                ctx.render()?;
            }
            return Ok(());
        }
        if self.collapsed_area.contains(event.into()) {
            if click {
                self.hits.set_focus_filters(true);
                ctx.render()?;
            }
            return Ok(());
        }
        if self.queue_area.contains(event.into()) {
            if click && self.hits.focus_filters() {
                self.hits.set_focus_filters(false);
                ctx.render()?;
            }
            return self.queue.handle_mouse_event(event, ctx);
        }
        self.hits.handle_mouse_event(event, ctx)?;
        if self.hits.take_commit() {
            self.apply(ctx);
            ctx.render()?;
        }
        Ok(())
    }

    fn resize(&mut self, area: Rect, ctx: &Ctx) -> Result<()> {
        self.queue.resize(area, ctx)
    }
}
