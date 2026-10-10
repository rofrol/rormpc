//! A view-only projection of Queue. Row positions never become MPD positions;
//! requests and forecast patches go to mpd-player, which alone publishes
//! priorities. The ordinary Queue Dir stays in physical queue order.

use std::{
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Row, TableState},
};
use rmpc_mpd::{commands::Song, mpd_client::MpdClient};

use crate::{
    config::keys::{CommonAction, QueueActions},
    ctx::Ctx,
    shared::{
        events::AppEvent,
        id::{self, Id},
        keys::ActionEvent,
        macros::{modal, status_info, status_warn},
        mouse_event::{MouseEvent, MouseEventKind, calculate_scrollbar_position},
    },
    ui::{
        dirstack::DirState,
        input::{BufferId, InputResultEvent},
        modals::menu::modal::MenuModal,
        rormpc_player::{self, ShuffleState},
        rormpc_upnext::{self, Waiting},
        song_ext::SongExt,
        widgets::virtualized_table::VirtualizedTable,
    },
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Turn {
    Past(i8),
    Current,
    Request(usize),
    /// Music's "Up next · N" row above the requests (no song; Enter or the menu on it clears Up next)
    Header,
    Forecast(usize),
    /// a queued song outside the forecast; shown only as a `/` match
    Pool,
}

impl Turn {
    fn key(self) -> (u8, i64) {
        match self {
            Self::Past(n) => (0, i64::from(n)),
            Self::Current => (1, 0),
            Self::Request(n) => (2, n.cast_signed() as i64),
            Self::Header => (2, 0),
            Self::Forecast(n) => (3, n.cast_signed() as i64),
            Self::Pool => (4, 0),
        }
    }

    fn label(self) -> String {
        match self {
            Self::Past(n) => n.to_string(),
            Self::Current => "0 ▶".to_owned(),
            Self::Request(n) => format!("↑{n}"),
            Self::Header => String::new(),
            Self::Forecast(n) => n.to_string(),
            Self::Pool => "·".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanRow {
    pub id: Option<u32>,
    pub turn: Turn,
}

pub fn project(
    queue: &[Song],
    current: Option<u32>,
    requests: &[Waiting],
    sh: &ShuffleState,
) -> Vec<PlanRow> {
    let by_id: HashMap<u32, &str> = queue.iter().map(|s| (s.id, s.file.as_str())).collect();
    let mut turns = HashMap::new();
    // One row per queue id, even if a history entry also appears in the current
    // plan.
    for (n, past) in sh.history.iter().rev().take(2).enumerate() {
        if let Some(id) = past.id
            && by_id.get(&id).copied() == Some(past.file.as_str())
        {
            turns.entry(id).or_insert(Turn::Past(-((n + 1) as i8)));
        }
    }
    for (n, entry) in sh.plan.iter().enumerate().rev() {
        if by_id.get(&entry.id).copied() == Some(entry.file.as_str()) {
            turns.insert(entry.id, Turn::Forecast(n + 1));
        }
    }
    for (n, entry) in requests.iter().enumerate().rev() {
        if by_id.get(&entry.id).copied() == Some(entry.file.as_str()) {
            turns.insert(entry.id, Turn::Request(n + 1));
        }
    }
    if let Some(id) = current
        && by_id.contains_key(&id)
    {
        turns.insert(id, Turn::Current);
    }
    let mut rows: Vec<_> = queue
        .iter()
        .enumerate()
        .map(|(pos, s)| {
            (pos, PlanRow {
                id: Some(s.id),
                turn: turns.get(&s.id).copied().unwrap_or(Turn::Pool),
            })
        })
        .collect();
    rows.sort_by_key(|(pos, row)| (row.turn.key(), *pos));
    rows.into_iter().map(|(_, row)| row).collect()
}

pub fn stale(sh: &ShuffleState, present: bool, now: f64) -> bool {
    !present
        || !sh.updated_at.is_finite()
        || !now.is_finite()
        || sh.plan_version.is_empty()
        || sh.publish_error.is_some()
        || sh.updated_at <= 0.0
        || sh.updated_at > now
        || now - sh.updated_at >= rormpc_player::FORECAST_FRESH_SECONDS
}

/// The "Up next · N" header row's text, ruled to `width`.
pub fn header_line(n: usize, width: u16) -> String {
    let text = format!(" ── {} ", rormpc_upnext::block_label(n));
    let fill = usize::from(width).saturating_sub(text.chars().count());
    format!("{text}{}", "─".repeat(fill))
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64()
}

#[cfg(test)]
mod tests;

#[derive(Debug)]
#[allow(clippy::struct_excessive_bools)] // independent view flags: selection, typing, staleness, header row
pub struct PlanView {
    pub rows: Vec<PlanRow>,
    pub selected_id: Option<u32>,
    pub marked: BTreeSet<u32>,
    state: DirState<TableState>,
    area: Rect,
    scrollbar: Rect,
    like_cell: Option<Rect>,
    /// the song ID whose like cell the mouse is over, from the last paint (underlined: it can be clicked)
    hover_like: Option<u32>,
    rendered_rows: Vec<Option<(u32, String)>>,
    known_files: HashMap<u32, String>,
    invalidated_selection: bool,
    /// the cursor is on the "Up next · N" row (it has no song id)
    on_header: bool,
    /// the screen row (from the table's top) the header was painted on, for the mouse
    rendered_header: Option<usize>,
    buffer: BufferId,
    pub typing: bool,
    query: String,
    saved_id: Option<u32>,
    snapshot: ShuffleState,
    pub is_stale: bool,
    pending: Option<(String, Instant)>,
    ack_timer: Id,
    freshness_timer: Id,
    scheduled_freshness: f64,
    /// the title's name: Play's table follows `w`, the Queue pane's `o`
    pub name: &'static str,
}

impl PlanView {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            selected_id: None,
            marked: BTreeSet::new(),
            state: DirState::default(),
            area: Rect::default(),
            scrollbar: Rect::default(),
            like_cell: None,
            hover_like: None,
            rendered_rows: Vec::new(),
            known_files: HashMap::new(),
            invalidated_selection: false,
            on_header: false,
            rendered_header: None,
            buffer: BufferId::new(),
            typing: false,
            query: String::new(),
            saved_id: None,
            snapshot: ShuffleState::default(),
            is_stale: true,
            pending: None,
            ack_timer: id::new(),
            freshness_timer: id::new(),
            scheduled_freshness: 0.0,
            name: "Queue · plan (o: queue)",
        }
    }

    /// The `/` search narrows the rows (typed or kept after Enter).
    pub fn has_query(&self) -> bool {
        !self.query.is_empty()
    }

    pub fn refresh(&mut self, ctx: &Ctx, snap: bool) {
        let top = self.rows.iter().skip(self.state.offset()).find_map(|r| r.id);
        let screen_row = self.state.get_selected().unwrap_or(0).saturating_sub(self.state.offset());
        let previous_index = self.state.get_selected().unwrap_or(0);
        // A missed/coalesced watcher event must not turn a published
        // acknowledgement into a false timeout.
        if self
            .pending
            .as_ref()
            .is_some_and(|(_, started)| started.elapsed() >= rormpc_player::ANSWER_TIMEOUT)
        {
            rormpc_player::invalidate_shuffle_state();
        }
        self.snapshot = rormpc_player::shuffle_state();
        let present = ctx.player_present.load(std::sync::atomic::Ordering::Relaxed);
        #[cfg(not(test))]
        let present = present
            && crate::ui::rormpc_process::alive(
                self.snapshot.pid,
                &self.snapshot.plan_version,
                &ctx.app_event_sender,
            );
        let current_time = now();
        self.is_stale = stale(&self.snapshot, present, current_time);
        if let Some((token, started)) = &self.pending {
            if let Some(ack) = &self.snapshot.ack
                && ack.token == *token
            {
                if ack.ok {
                    status_info!("Forecast patch confirmed (until the next draw)");
                } else {
                    status_warn!(
                        "Forecast patch rejected: {}",
                        ack.error.as_deref().unwrap_or("unknown error")
                    );
                }
                self.pending = None;
                ctx.scheduler.cancel(self.ack_timer);
            } else if started.elapsed() >= rormpc_player::ANSWER_TIMEOUT {
                status_warn!("mpd-player did not confirm the forecast patch; no automatic retry");
                self.pending = None;
            }
        }
        if !self.is_stale {
            let deadline = self.snapshot.updated_at + rormpc_player::FORECAST_FRESH_SECONDS;
            if deadline.to_bits() != self.scheduled_freshness.to_bits() {
                self.scheduled_freshness = deadline;
                ctx.scheduler.schedule_replace(
                    self.freshness_timer,
                    Duration::from_secs_f64((deadline - current_time).max(0.0)),
                    |(tx, _)| Ok(tx.send(AppEvent::RequestRender)?),
                );
            }
        }
        let requests = rormpc_upnext::waiting(ctx);
        let mut seen = BTreeSet::new();
        self.is_stale |= self.snapshot.plan.iter().any(|e| {
            !seen.insert(e.id)
                || ctx.status.songid == Some(e.id)
                || !ctx.queue.iter().any(|s| s.id == e.id && s.file == e.file)
                || requests.iter().any(|r| r.id == e.id && r.file == e.file)
        });
        let mut rows = project(&ctx.queue, ctx.status.songid, &requests, &self.snapshot);
        if self.query.is_empty() {
            rows.retain(|r| r.turn != Turn::Pool);
            // the requests' block gets its header row, shown only while a request is in it
            if let Some(at) = rows.iter().position(|r| matches!(r.turn, Turn::Request(_))) {
                rows.insert(at, PlanRow { id: None, turn: Turn::Header });
            }
        } else {
            let found = crate::ui::panes::queue::find_matches(&ctx.queue, &self.query);
            let matches: BTreeSet<_> = found.rows.iter().map(|i| ctx.queue[*i].id).collect();
            // Pool songs (outside the forecast) appear only as matches, after
            // the forecast, so their row actions stay reachable.
            rows.retain(|r| r.id.is_some_and(|id| matches.contains(&id)));
        }
        let files: HashMap<_, _> = ctx.queue.iter().map(|s| (s.id, s.file.as_str())).collect();
        if let Some(id) = self.selected_id
            && files
                .get(&id)
                .is_none_or(|file| self.known_files.get(&id).is_some_and(|old| old != file))
        {
            self.invalidated_selection = true;
        }
        self.marked.retain(|id| {
            files.get(id).is_some_and(|file| self.known_files.get(id).is_none_or(|old| old == file))
        });
        self.known_files.retain(|id, _| files.contains_key(id));
        for (id, file) in files {
            self.known_files
                .entry(id)
                .and_modify(|old| {
                    if old != file {
                        file.clone_into(old);
                    }
                })
                .or_insert_with(|| file.to_owned());
        }
        self.rows = rows;
        self.state.set_content_and_viewport_len(self.rows.len(), usize::from(self.area.height));
        let selected = if snap {
            None
        } else if self.on_header {
            self.rows.iter().position(|r| r.turn == Turn::Header)
        } else {
            self.selected_id.and_then(|id| self.rows.iter().position(|r| r.id == Some(id)))
        };
        let selected = selected.or_else(|| {
            self.rows
                .iter()
                .enumerate()
                .skip(if snap { 0 } else { previous_index.min(self.rows.len().saturating_sub(1)) })
                .find(|(_, r)| r.id.is_some())
                .map(|(i, _)| i)
                .or_else(|| self.rows.iter().position(|r| r.id.is_some()))
        });
        self.state.select(selected, 0);
        if let Some(selected) = selected {
            let anchor = top.and_then(|id| self.rows.iter().position(|r| r.id == Some(id)));
            let offset = anchor
                .filter(|i| selected >= *i && selected < *i + usize::from(self.area.height))
                .unwrap_or_else(|| selected.saturating_sub(screen_row));
            self.state.set_offset(
                offset.min(self.rows.len().saturating_sub(usize::from(self.area.height))),
            );
        }
        self.remember_selection();
        if snap {
            self.invalidated_selection = false;
        } // explicit search input chose the new matching row
    }

    fn remember_selection(&mut self) {
        let row = self.state.get_selected().and_then(|i| self.rows.get(i));
        self.on_header = row.is_some_and(|r| r.turn == Turn::Header);
        self.selected_id = row.and_then(|r| r.id);
    }

    pub fn select_id(&mut self, id: Option<u32>) {
        self.on_header = false;
        self.selected_id = id;
        self.known_files.clear();
        self.invalidated_selection = false;
    }

    pub fn selected_for_action(&self) -> Option<u32> {
        if self.invalidated_selection { None } else { self.selected_id }
    }

    fn selected_turn(&self) -> Option<Turn> {
        if self.on_header {
            return Some(Turn::Header);
        }
        self.selected_id.and_then(|id| self.rows.iter().find(|r| r.id == Some(id))).map(|r| r.turn)
    }

    /// How many request rows the "Up next · N" header stands above.
    fn requests_shown(&self) -> usize {
        self.rows.iter().filter(|r| matches!(r.turn, Turn::Request(_))).count()
    }

    /// `gu`: the cursor on the "Up next · N" header row; false when no request is waiting.
    pub fn jump_to_block(&mut self, ctx: &Ctx) -> bool {
        self.refresh(ctx, false);
        let Some(i) = self.rows.iter().position(|r| r.turn == Turn::Header) else { return false };
        self.state.select(Some(i), ctx.config.scrolloff);
        self.remember_selection();
        self.invalidated_selection = false;
        true
    }

    fn play(&self, ctx: &Ctx) {
        if self.on_header {
            return rormpc_upnext::open_block_menu(ctx, self.requests_shown());
        }
        if let Some(id) = self.selected_for_action() {
            if matches!(self.selected_turn(), Some(Turn::Request(_))) {
                rormpc_upnext::play_entry(ctx, id);
            } else {
                ctx.command(move |_, client| Ok(client.play_id(id)?));
            }
        }
    }

    fn search(&mut self, ctx: &Ctx) {
        if !self.typing {
            self.saved_id = self.selected_id;
            ctx.input.create_buffer(self.buffer, None);
        }
        self.typing = true;
        ctx.input.insert_mode(self.buffer);
    }

    pub fn insert(&mut self, kind: &InputResultEvent, ctx: &Ctx) -> Result<()> {
        match kind {
            InputResultEvent::Push | InputResultEvent::Pop => {
                self.query = ctx.input.value(self.buffer);
                self.refresh(ctx, true);
            }
            InputResultEvent::Confirm => {
                self.play(ctx);
                self.typing = false;
                self.query.clear();
                ctx.input.clear_buffer(self.buffer);
                self.refresh(ctx, false);
            }
            InputResultEvent::Cancel => {
                self.typing = false;
                self.query.clear();
                ctx.input.clear_buffer(self.buffer);
                self.selected_id = self.saved_id;
                self.refresh(ctx, false);
            }
            InputResultEvent::NoChange => {}
        }
        Ok(ctx.render()?)
    }

    pub fn insert_nav(&mut self, down: bool, ctx: &Ctx) -> Result<()> {
        if down {
            self.state.next(ctx.config.scrolloff, ctx.config.wrap_navigation);
        } else {
            self.state.prev(ctx.config.scrolloff, ctx.config.wrap_navigation);
        }
        self.remember_selection();
        self.invalidated_selection = false;
        Ok(ctx.render()?)
    }

    fn move_selected(&mut self, delta: isize, ctx: &Ctx) {
        let Some(id) = self.selected_for_action() else { return };
        match self.selected_turn() {
            Some(Turn::Request(n)) => {
                let waiting = rormpc_upnext::waiting(ctx);
                let target = n.cast_signed() + delta;
                let full = project(&ctx.queue, ctx.status.songid, &waiting, &self.snapshot);
                if target > 0 && full.iter().any(|r| r.turn == Turn::Request(target as usize)) {
                    rormpc_upnext::move_entry(ctx, id, delta);
                }
            }
            Some(Turn::Forecast(n)) if !self.is_stale => {
                if self.pending.is_some() {
                    return status_info!("Waiting for the forecast patch acknowledgement");
                }
                let next = n.cast_signed() - 1 + delta;
                let Some(other) =
                    usize::try_from(next).ok().and_then(|i| self.snapshot.plan.get(i))
                else {
                    return;
                };
                let token =
                    format!("{}-{}-{}", self.snapshot.plan_version, std::process::id(), *id::new());
                let msg = format!(
                    "shuffle swap {} {id} {} {token}",
                    self.snapshot.plan_version, other.id
                );
                self.pending = Some((token, Instant::now()));
                ctx.scheduler.schedule_replace(
                    self.ack_timer,
                    rormpc_player::ANSWER_TIMEOUT,
                    |(tx, _)| Ok(tx.send(AppEvent::RequestRender)?),
                );
                ctx.command(move |_, client| {
                    if !client.channels()?.0.iter().any(|c| c == rormpc_player::CHANNEL) {
                        status_warn!("mpd-player is not running: forecast not changed");
                        return Ok(());
                    }
                    Ok(client.send_message(rormpc_player::CHANNEL, &msg)?)
                });
            }
            Some(Turn::Forecast(_)) => status_warn!("Stale forecast: cannot patch it"),
            _ => status_info!(
                "J/K move only requests or adjacent forecast slots, never across sections"
            ),
        }
    }

    fn menu(&self, ctx: &Ctx) {
        let Some(id) = self.selected_for_action() else { return };
        let Some(song) = ctx.queue.iter().find(|s| s.id == id) else { return };
        let file = song.file.clone();
        let request = matches!(self.selected_turn(), Some(Turn::Request(_)));
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |section| {
                let section = section.item("Play now", move |ctx| {
                    if request {
                        rormpc_upnext::play_entry(ctx, id);
                    } else {
                        ctx.command(move |_, client| Ok(client.play_id(id)?));
                    }
                    Ok(())
                });
                Some(if request {
                    section
                        .item("Make next", move |ctx| {
                            rormpc_upnext::make_next(ctx, id);
                            Ok(())
                        })
                        .item("Remove from Up next", move |ctx| {
                            rormpc_upnext::remove(ctx, id);
                            Ok(())
                        })
                } else {
                    section.item("Play next", move |ctx| {
                        rormpc_upnext::play_next(ctx, vec![file.clone()]);
                        Ok(())
                    })
                })
            })
            .build();
        modal!(ctx, menu);
    }

    /// False only for ID/file-based actions which the ordinary Queue handler
    /// can safely execute after mapping.
    pub fn action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<bool> {
        self.refresh(ctx, false);
        if let Some(action) = event.claim_queue().cloned() {
            match action {
                QueueActions::Play => self.play(ctx),
                QueueActions::Find => self.search(ctx),
                QueueActions::JumpToCurrent => {
                    if let Some(i) =
                        self.rows.iter().position(|r| r.id == ctx.status.songid && r.id.is_some())
                    {
                        self.state.select(Some(i), usize::MAX);
                        self.remember_selection();
                        self.invalidated_selection = false;
                    }
                }
                QueueActions::Delete if self.on_header && self.marked.is_empty() => {
                    status_info!("d removes a request; D here clears Up next");
                }
                QueueActions::Delete => {
                    let ids: Vec<_> = if self.marked.is_empty() {
                        self.selected_for_action().into_iter().collect()
                    } else {
                        self.marked.iter().copied().collect()
                    };
                    // a request leaves Up next (an added song leaves the queue too, a source song stays)
                    let waiting = rormpc_upnext::up_next_ids();
                    let (requests, ids): (Vec<_>, Vec<_>) = ids.into_iter().partition(|id| waiting.contains(id));
                    for id in requests {
                        rormpc_upnext::remove(ctx, id);
                    }
                    if !ids.is_empty() {
                        ctx.command(move |_, client| {
                            for id in ids {
                                client.delete_id(id)?;
                            }
                            Ok(())
                        });
                    }
                }
                QueueActions::DeleteAll if self.on_header => {
                    rormpc_upnext::confirm_clear(ctx, self.requests_shown());
                }
                QueueActions::DeleteAll => {
                    event.abandon();
                    return Ok(false);
                } // existing explicit confirmation
                // by file (rormpc exceptions): the ordinary handler on the mapped selection
                QueueActions::PinSong | QueueActions::ExcludeSong => {
                    event.abandon();
                    return Ok(false);
                }
                _ => status_info!(
                    "Plan view does not sort or shuffle the MPD queue; o returns to queue order"
                ),
            }
            ctx.render()?;
            return Ok(true);
        }
        let Some(action) = event.claim_common().cloned() else { return Ok(false) };
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        match &action {
            CommonAction::Up | CommonAction::PreviousResult => self.state.prev(scrolloff, wrap),
            CommonAction::Down | CommonAction::NextResult => self.state.next(scrolloff, wrap),
            CommonAction::UpHalf => self.state.prev_half_viewport(scrolloff),
            CommonAction::DownHalf => self.state.next_half_viewport(scrolloff),
            CommonAction::PageUp => self.state.prev_viewport(scrolloff),
            CommonAction::PageDown => self.state.next_viewport(scrolloff),
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::ScrollFocusedToTop => self.state.scroll_focused_to_top(scrolloff),
            CommonAction::ScrollFocusedToMiddle => self.state.scroll_focused_to_middle(scrolloff),
            CommonAction::ScrollFocusedToBottom => self.state.scroll_focused_to_bottom(scrolloff),
            CommonAction::MoveUp => self.move_selected(-1, ctx),
            CommonAction::MoveDown => self.move_selected(1, ctx),
            CommonAction::Confirm => self.play(ctx),
            CommonAction::EnterSearch | CommonAction::FocusInput => self.search(ctx),
            CommonAction::Close if !self.query.is_empty() => {
                self.query.clear();
                ctx.input.clear_buffer(self.buffer);
                self.selected_id = self.saved_id;
                self.refresh(ctx, false);
            }
            CommonAction::Close if !self.marked.is_empty() => self.marked.clear(),
            CommonAction::Select => {
                if let Some(id) = self.selected_for_action() {
                    if !self.marked.remove(&id) {
                        self.marked.insert(id);
                    }
                    self.state.next(scrolloff, wrap);
                }
            }
            CommonAction::InvertSelection => {
                for id in self.rows.iter().filter_map(|r| r.id) {
                    if !self.marked.remove(&id) {
                        self.marked.insert(id);
                    }
                }
            }
            CommonAction::ContextMenu if self.on_header => {
                rormpc_upnext::open_block_menu(ctx, self.requests_shown());
            }
            CommonAction::ContextMenu if matches!(self.selected_turn(), Some(Turn::Request(_))) => {
                self.menu(ctx);
            }
            CommonAction::ShowInfo
            | CommonAction::Rate { .. }
            | CommonAction::CopyToClipboard { .. }
            | CommonAction::Save { .. }
            | CommonAction::DeleteFromPlaylist { .. }
            | CommonAction::AddOptions { .. }
            | CommonAction::ContextMenu
            | CommonAction::PaneUp
            | CommonAction::PaneDown
            | CommonAction::PaneLeft
            | CommonAction::PaneRight => {
                event.abandon();
                return Ok(false);
            }
            _ => {
                event.abandon();
                return Ok(false);
            }
        }
        self.remember_selection();
        if matches!(
            action,
            CommonAction::Up
                | CommonAction::Down
                | CommonAction::UpHalf
                | CommonAction::DownHalf
                | CommonAction::PageUp
                | CommonAction::PageDown
                | CommonAction::Top
                | CommonAction::Bottom
                | CommonAction::NextResult
                | CommonAction::PreviousResult
        ) {
            self.invalidated_selection = false;
        }
        ctx.render()?;
        Ok(true)
    }

    fn remember_rendered(&mut self, ctx: &Ctx) {
        self.invalidated_selection = false; // the fallback cursor now names a song actually painted for the user
        self.rendered_rows = self
            .rows
            .iter()
            .skip(self.state.offset())
            .take(usize::from(self.area.height))
            .map(|r| {
                r.id.and_then(|id| {
                    ctx.queue.iter().find(|s| s.id == id).map(|s| (id, s.file.clone()))
                })
            })
            .collect();
    }

    pub fn mouse(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<bool> {
        // The pointer belongs to the last paint, not a forecast recomputed
        // between that paint and this event.
        let target = self
            .area
            .contains(event.into())
            .then(|| self.rendered_rows.get(usize::from(event.y.saturating_sub(self.area.y))))
            .flatten()
            .cloned()
            .flatten();
        let on_header = self.area.contains(event.into())
            && self.rendered_header == Some(usize::from(event.y.saturating_sub(self.area.y)));
        // a move only changes the like cell's hover; it renders when that changed
        if matches!(event.kind, MouseEventKind::Moved) {
            let hover = target
                .as_ref()
                .filter(|_| self.like_cell.is_some_and(|r| r.contains(event.into())))
                .map(|(id, _)| *id);
            if hover != self.hover_like {
                self.hover_like = hover;
                ctx.render()?;
            }
            return Ok(true);
        }
        self.refresh(ctx, false);
        if on_header
            && matches!(
                event.kind,
                MouseEventKind::LeftClick | MouseEventKind::DoubleClick | MouseEventKind::RightClick
            )
        {
            if self.jump_to_block(ctx) && !matches!(event.kind, MouseEventKind::LeftClick) {
                rormpc_upnext::open_block_menu(ctx, self.requests_shown());
            }
            ctx.render()?;
            return Ok(true);
        }
        if self.scrollbar.contains(event.into())
            && matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::Drag { .. })
        {
            if let Some(p) = calculate_scrollbar_position(event, self.scrollbar) {
                self.state.scroll_to(p, ctx.config.scrolloff);
                self.remember_selection();
            }
            ctx.render()?;
            return Ok(true);
        }
        if !self.area.contains(event.into()) {
            return Ok(true);
        }
        let idx = target.as_ref().and_then(|(id, file)| {
            ctx.queue
                .iter()
                .any(|s| s.id == *id && s.file == *file)
                .then(|| self.rows.iter().position(|r| r.id == Some(*id)))
                .flatten()
        });
        if self.like_cell.is_some_and(|r| r.contains(event.into()))
            && matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::DoubleClick)
        {
            if let Some(song) = idx
                .and_then(|i| self.rows.get(i))
                .and_then(|r| r.id)
                .and_then(|id| ctx.queue.iter().find(|s| s.id == id))
            {
                let liked = ctx
                    .song_stickers(&song.file)
                    .and_then(|st| st.get("like"))
                    .is_some_and(|v| v == "2");
                crate::ui::rormpc_actions::set_like(
                    ctx,
                    song.file.clone(),
                    if liked { "1" } else { "2" },
                );
            }
            return Ok(true);
        }
        match event.kind {
            MouseEventKind::LeftClick
            | MouseEventKind::DoubleClick
            | MouseEventKind::RightClick
            | MouseEventKind::MiddleClick => {
                let Some(i) = idx.filter(|i| self.rows.get(*i).is_some_and(|r| r.id.is_some()))
                else {
                    return Ok(true);
                };
                self.state.select(Some(i), ctx.config.scrolloff);
                self.remember_selection();
                self.invalidated_selection = false;
                match event.kind {
                    MouseEventKind::DoubleClick => self.play(ctx),
                    MouseEventKind::RightClick
                        if matches!(self.selected_turn(), Some(Turn::Request(_))) =>
                    {
                        self.menu(ctx);
                    }
                    // the ordinary ID-safe Queue context menu
                    MouseEventKind::RightClick => return Ok(false),
                    MouseEventKind::MiddleClick => {
                        if let Some(id) = self.selected_id {
                            ctx.command(move |_, client| Ok(client.delete_id(id)?));
                        }
                    }
                    _ => {}
                }
            }
            MouseEventKind::ScrollDown => {
                self.hover_like = None; // another song is under the pointer now: the next move sets it again
                self.state.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff);
                self.remember_selection();
            }
            MouseEventKind::ScrollUp => {
                self.hover_like = None;
                self.state.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff);
                self.remember_selection();
            }
            _ => return Ok(true),
        }
        ctx.render()?;
        Ok(true)
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let [title, body, search] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(u16::from(self.typing || !self.query.is_empty())),
        ])
        .areas(area);
        let [table, scrollbar] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(u16::from(ctx.config.theme.scrollbar.is_some())),
        ])
        .areas(body);
        self.area = table;
        self.scrollbar = scrollbar;
        self.refresh(ctx, false);
        let age = (now() - self.snapshot.updated_at).max(0.0) / 60.0;
        let freshness = if self.is_stale {
            if self.snapshot.updated_at <= 0.0 {
                "stale · unknown age".to_owned()
            } else {
                format!("stale · {:.0}m", age.floor())
            }
        } else {
            "forecast, redrawn every song".to_owned()
        };
        let pending = if self.pending.is_some() { " · awaiting patch" } else { "" };
        frame.render_widget(
            Line::from(format!("{} · {freshness}{pending}", self.name))
                .style(ctx.config.theme.preview_label_style),
            title,
        );
        let formats = &ctx.config.theme.song_table_format;
        let has_next = formats.iter().any(|f| format!("{:?}", f.prop).contains("ShuffleNext"));
        let marker_col =
            formats.iter().position(|f| !format!("{:?}", f.prop).contains("ShuffleNext"));
        let widths: Vec<_> = formats.iter().map(|f| f.width.into_constraint(0)).collect();
        let cells = Layout::horizontal(widths.clone()).spacing(1).split(table);
        let like_idx =
            formats.iter().position(|f| format!("{:?}", f.prop).contains("Sticker(\"like\")"));
        self.like_cell = like_idx.and_then(|i| cells.get(i).copied());
        let hover_like = self.hover_like;
        let selected = self.selected_id;
        let by_id: HashMap<_, _> = ctx.queue.iter().map(|s| (s.id, s)).collect();
        let widget = VirtualizedTable::new(&self.rows).column_widths(widths).map_fn(|_, row| {
            let Some(song) = row.id.and_then(|id| by_id.get(&id).copied()) else {
                return Row::new(Vec::<Line>::new());
            };
            let columns = formats
                .iter()
                .enumerate()
                .map(|(i, format)| {
                    let is_next = format!("{:?}", format.prop).contains("ShuffleNext");
                    let marked = Some(i) == marker_col && self.marked.contains(&song.id);
                    // a pin ✚ or an exclusion ⊘ names this file
                    let except = (Some(i) == marker_col)
                        .then(|| crate::ui::rormpc_exceptions::mark_for(ctx, &song.file))
                        .flatten();
                    let prefix_width = if i == 0 && !has_next { 4 } else { 0 }
                        + if except.is_some() { 2 } else { 0 }
                        + if marked {
                            Line::from(ctx.config.theme.symbols.marker.as_str()).width()
                        } else {
                            0
                        };
                    let mut line = if is_next {
                        Line::from(row.turn.label())
                    } else {
                        song.as_line_ellipsized(
                            &format.prop,
                            usize::from(cells[i].width).saturating_sub(prefix_width),
                            &ctx.config.theme.symbols,
                            &ctx.config.theme.format_tag_separator,
                            ctx.config.theme.multiple_tag_resolution_strategy,
                            ctx,
                        )
                        .unwrap_or_default()
                    };
                    if let Some(kind) = except {
                        line.spans.insert(0, Span::raw(format!("{} ", kind.mark())));
                    }
                    if i == 0 && !has_next {
                        let marker_style = if self.is_stale && matches!(row.turn, Turn::Forecast(_))
                        {
                            Style::default().add_modifier(Modifier::DIM)
                        } else {
                            Style::default()
                        };
                        line.spans.insert(
                            0,
                            Span::styled(format!("{:>3} ", row.turn.label()), marker_style),
                        );
                    }
                    if marked {
                        line.spans.insert(0, Span::raw(ctx.config.theme.symbols.marker.clone()));
                    }
                    if self.is_stale && matches!(row.turn, Turn::Forecast(_)) && is_next {
                        line.style = line.style.add_modifier(Modifier::DIM);
                    }
                    if Some(i) == like_idx && hover_like == Some(song.id) {
                        line = crate::ui::rormpc_actions::hovered_like(line);
                    }
                    line
                })
                .collect::<Vec<_>>();
            let mut style = if selected == row.id {
                ctx.config.theme.current_item_style
            } else if row.turn == Turn::Current {
                ctx.config.theme.highlighted_item_style
            } else {
                ctx.config.as_text_style()
            };
            if matches!(row.turn, Turn::Past(_)) {
                style = style.add_modifier(Modifier::DIM);
            }
            Row::new(columns).style(style)
        });
        frame.render_stateful_widget(widget, table, &mut self.state);
        // the header row spans the table: drawn over its empty row
        self.rendered_header = self
            .rows
            .iter()
            .position(|r| r.turn == Turn::Header)
            .and_then(|i| i.checked_sub(self.state.offset()))
            .filter(|row| *row < usize::from(table.height));
        if let Some(row) = self.rendered_header {
            let style = if self.on_header {
                ctx.config.theme.current_item_style
            } else {
                ctx.config.theme.preview_label_style.add_modifier(Modifier::BOLD)
            };
            let line = header_line(self.requests_shown(), table.width);
            let at = Rect { y: table.y + row as u16, height: 1, ..table };
            frame.render_widget(Line::from(line).style(style), at);
        }
        if let Some(widget) = ctx.config.as_styled_scrollbar() {
            frame.render_stateful_widget(widget, scrollbar, self.state.as_scrollbar_state_ref());
        }
        self.remember_rendered(ctx);
        if search.height > 0 {
            frame.render_widget(
                Line::from(format!(
                    "FILTER / {}{} · {} shown{}",
                    self.query,
                    if self.typing { "▏" } else { "" },
                    self.rows.iter().filter(|r| r.id.is_some()).count(),
                    match self.rows.iter().filter(|r| r.turn == Turn::Pool).count() {
                        0 => String::new(),
                        n => format!(" · {n} · in the pool, not in the forecast"),
                    }
                )),
                search,
            );
        }
    }
}
