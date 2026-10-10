//! rormpc: the Play pane (plans/combined-view.md, phases 3 and 3b): Queue, Hits, Shuffle and the browsing tabs
//! in one tab.
//!
//! - Normal mode (weighted off): MPD's queue in its order; the Hits filter column is collapsed to one line naming
//!   the source, `h` or a click opens it.
//! - Weighted mode (`w`): the plan projection (past plays, `0 ▶`, Up next and the forecast; a pool song shows
//!   only as a `/` match) with the filter column open. Turning weighted off keeps the queue as it is.
//! - A filter change prepares a preview (`hits` into its own file, MPD untouched): the table shows it under a
//!   banner with its counts; `a` (or Apply) plays it, Esc drops it and shows the playing source again. See
//!   `rormpc_play` for Apply's confirmation and race rules.
//! - The header line always says what plays and how: `▶ Playing from: … · weighted · round 12/84`, and the open
//!   smart list (`Smart list: 80s party`, "changed" once the filters differ from its rules).
//! - Smart lists (`rormpc_smartlists`): `S` saves the filters, `L` picks a list, a previous source or a playlist;
//!   a picked list loads as a preview, `a` plays it.
//! - Phase 3b (decided by the user 2026-10-10): the left column switches Filters | Browse (`B`, or a click on
//!   the switch). Browse has the groupings Artists · Album artists · Albums · Folders · Lists (`[` `]`, the chip
//!   row, or 5-9 from anywhere); each is the browser pane of the old tab, owned here and created on first use, with
//!   its own query target (`PaneType::PlayBrowse`). In Browse `P` plays the selection replacing the queue (in its
//!   order, Apply's confirmation rule), `t` puts it into Up next, `a`/`A` append (a Hits source takes the appended
//!   songs into its round). A stored playlist opened in Lists is edited in a panel over Play (moves, removals and
//!   renames are saved at once; generated playlists are read-only), and the Live playlists inbox is a panel too
//!   (`0`, `gl`, or the `Live N` badge in the header). An unapplied preview stays open through all of it.
//! - The Deleted pane is a panel over Music too (decided by the user 2026-10-10): `gd`, or the `Deleted ! N` badge,
//!   shown only while deletions have failed or unresolved steps.
//!
//! It is composed, not copied: the filter column and the preview table are a `HitsPane` in Play mode, the table
//! is a `QueuePane` whose view follows the weighted shuffle, Browse's groupings are the browser panes.

use std::{collections::HashMap, fmt::Write as _};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Color, Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::{
    Pane,
    PaneContainer,
    directories::DirectoriesPane,
    hits::{HitsBody, HitsPane},
    deleted::DeletedPane,
    live_playlists::LivePlaylistsPane,
    playlists::PlaylistsPane,
    queue::QueuePane,
    tag_browser::TagBrowserPane,
};
use crate::{
    MpdQueryResult,
    config::{
        keys::{
            CommonAction,
            QueueActions,
            actions::{AddKind, Position},
        },
        tabs::{PaneType, PlayGrouping, PlayView},
    },
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        macros::{modal, status_error, status_info, status_warn},
        mouse_event::{MouseEvent, MouseEventKind},
        mpd_client_ext::{MpdClientExt as _, MpdDelete},
    },
    ui::{
        UiEvent,
        browser::BrowserPane,
        dir_or_song::DirOrSong,
        input::InputResultEvent,
        modals::confirm_modal::{Action, ConfirmModal},
        rormpc_browse,
        rormpc_exceptions::{self, Kind},
        rormpc_play,
        rormpc_player,
        rormpc_smartlists::{self, Pick},
        rormpc_upnext::{self, HitsSource},
    },
};

/// Width of the filter column, as in the Hits pane.
const COLUMN_WIDTH: u16 = 24;
const APPLY_BUTTON: &str = "[ a Apply ]";

/// What Play's left column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Left {
    /// the Hits filter column (collapsed to one line in normal mode)
    Filters,
    /// the browser of a grouping
    Browse,
}

/// A Browse grouping: the browser pane of its old tab.
#[derive(Debug)]
enum Child {
    Tags(TagBrowserPane),
    Folders(DirectoriesPane),
    Lists(PlaylistsPane),
}

/// Run `$body` with `$b` bound to the browser pane inside a `Child`.
macro_rules! with_child {
    ($child:expr, $b:ident => $body:expr) => {
        match $child {
            Child::Tags($b) => $body,
            Child::Folders($b) => $body,
            Child::Lists($b) => $body,
        }
    };
}

/// Which panel is open over Play's body (the playlist editor follows Lists instead).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Overlay {
    None,
    Live,
    Deleted,
}

/// Areas of Browse's last render, for the mouse.
#[derive(Debug, Default)]
struct BrowseAreas {
    switch_filters: Rect,
    switch_browse: Rect,
    chips: Vec<(PlayGrouping, Rect)>,
    browser: Rect,
    /// the panel over Play (the Live inbox or the playlist editor)
    overlay: Rect,
    badge: Rect,
    /// the Deleted badge (empty while nothing needs attention)
    deleted_badge: Rect,
}

/// What an Apply waits for before it plays the preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyWait {
    No,
    /// a picked list's or previous source's Apply: its preview is being made
    Ready,
    /// Apply found the preview older than the library: `hits` runs again, the Apply that follows trusts it
    Recomputing,
}

#[derive(Debug)]
pub struct PlayPane {
    hits: HitsPane,
    queue: QueuePane,
    left: Left,
    /// Browse has the keys (else the table has them)
    browse_focus: bool,
    grouping: PlayGrouping,
    /// the groupings opened so far: each keeps its path, cursor and marks
    children: HashMap<PlayGrouping, Child>,
    /// the Live playlists inbox and the Deleted journal: their panes keep their place while closed
    live: LivePlaylistsPane,
    deleted: DeletedPane,
    overlay: Overlay,
    /// the rules the source being played was applied with (source.json), None for any other source; Esc brings
    /// the filters back to them (or to the defaults)
    baseline: Option<serde_json::Value>,
    /// the hash those rules give the filters: filters with another hash on screen are a preview
    baseline_hash: String,
    /// source.json's rules hash when `baseline` was taken (an Apply anywhere moves it); `synced` once taken
    source_hash: Option<String>,
    synced: bool,
    /// what the list picker and the Save modal hand back (`rormpc_smartlists::Pick`)
    inbox: rormpc_smartlists::Inbox,
    /// an Apply waiting for its preview (a picked list's or previous source's, or one that found the preview
    /// outdated): it plays once the preview is in
    wait: ApplyWait,
    /// areas of the last render, for the mouse
    collapsed_area: Rect,
    apply_area: Rect,
    queue_area: Rect,
    areas: BrowseAreas,
}

impl PlayPane {
    pub fn new(ctx: &Ctx) -> Self {
        let mut s = Self {
            hits: HitsPane::for_play(rormpc_play::PREVIEW_FILE.to_owned(), vec!["hits".to_owned()]),
            queue: QueuePane::new(ctx),
            left: Left::Filters,
            browse_focus: false,
            grouping: PlayGrouping::Albums,
            children: HashMap::new(),
            live: LivePlaylistsPane::new(),
            deleted: DeletedPane::new(),
            overlay: Overlay::None,
            baseline: None,
            baseline_hash: String::new(),
            source_hash: None,
            synced: false,
            inbox: rormpc_smartlists::Inbox::default(),
            wait: ApplyWait::No,
            collapsed_area: Rect::default(),
            apply_area: Rect::default(),
            queue_area: Rect::default(),
            areas: BrowseAreas::default(),
        };
        s.sync(ctx);
        rormpc_smartlists::load_in_background(ctx.app_event_sender.clone());
        s
    }

    /// The open smart list (id, name) with its current name, and whether the filters differ from its rules.
    fn open_list(&self) -> Option<(String, String, bool)> {
        let (id, name) = self.hits.open_list()?;
        let list = rormpc_smartlists::cached(&id);
        let name = list.as_ref().map_or(name, |l| l.name.clone());
        let changed = list
            .and_then(|l| l.args)
            .is_some_and(|args| Some(HitsPane::rules_hash_of(Some(&args))) != self.hits.rules_hash());
        Some((id, name, changed))
    }

    /// Take what the list picker or the Save modal left: load rules as a preview (Apply waits for it), or leave
    /// the open list.
    fn take_pick(&mut self) {
        let pick = self.inbox.lock().ok().and_then(|mut p| p.take());
        let (args, apply) = match pick {
            Some(Pick::Load(args)) => (args, false),
            Some(Pick::Apply(args)) => (args, true),
            Some(Pick::CloseList) => {
                self.hits.close_list();
                (serde_json::Value::Null, false)
            }
            None => (serde_json::Value::Null, false),
        };
        if !args.is_null() {
            if let Err(err) = self.hits.load_filters(&args) {
                status_error!("These rules cannot be loaded: {err}");
            } else if apply {
                self.wait = ApplyWait::Ready;
            } else {
                self.wait = ApplyWait::No;
                if !self.preview_active() {
                    status_info!("The queue already plays these rules");
                }
            }
        }
        rormpc_smartlists::set_open(self.open_list().map(|(id, name, _)| (id, name)));
    }

    /// A picked Apply waits for its preview: play it once `hits` wrote it (its run ends with a render).
    fn apply_if_ready(&mut self, ctx: &Ctx) {
        if self.wait != ApplyWait::No {
            let info = self.hits.preview_info();
            if info.error.is_some() || info.ready {
                self.apply(ctx); // it takes `wait`
            }
        }
    }

    /// `S`: save the filters on screen as a smart list.
    fn save_list(&self, ctx: &Ctx) {
        let info = self.hits.preview_info();
        let counts = (info.ready && !info.running).then(|| {
            let missing = info.matched.saturating_sub(info.files.len());
            format!("{} of {} ({missing} missing stay in the rules)", info.files.len(), info.matched)
        });
        let req = rormpc_smartlists::SaveRequest {
            rule_args: self.hits.rule_args(),
            lines: self.hits.rule_lines(),
            counts,
            open: self.open_list().map(|(id, name, _)| (id, name)),
        };
        rormpc_smartlists::open_save(ctx, std::sync::Arc::clone(&self.inbox), req);
    }

    /// `L`: the list picker.
    fn pick_list(&self, ctx: &Ctx) {
        let open = self.open_list().map(|(id, name, _)| (id, name));
        rormpc_smartlists::open_picker(ctx, &self.inbox, self.hits.rule_args(), open.as_ref());
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

    /// The left column is shown: Browse, or the filters while weighted, while they have the keys or show a
    /// preview.
    fn column_open(&self) -> bool {
        self.left == Left::Browse
            || Self::weighted()
            || self.hits.focus_filters()
            || self.preview_active()
            || self.hits.in_downloads()
    }

    /// The table right of the column is the Hits result (a preview, or the Downloads view), not the queue.
    fn shows_hits_table(&self) -> bool {
        self.preview_active() || self.hits.in_downloads()
    }

    /// Keys go to the Hits part (the column, or its table) rather than the queue.
    fn keys_to_hits(&self) -> bool {
        (self.column_open() && self.left == Left::Filters && self.hits.focus_filters()) || self.shows_hits_table()
    }

    // ---------------------------------------------------------------- Browse

    /// The browser of `grouping`, created (and asked for its root list) the first time.
    fn ensure_child(&mut self, grouping: PlayGrouping, ctx: &Ctx) -> &mut Child {
        self.children.entry(grouping).or_insert_with(|| {
            let target = PaneType::PlayBrowse(grouping);
            let [albums, artists, album_artists] = PaneContainer::tag_browser_levels(ctx);
            let mut child = match grouping {
                PlayGrouping::Artists => Child::Tags(TagBrowserPane::new(artists, target, ctx)),
                PlayGrouping::AlbumArtists => Child::Tags(TagBrowserPane::new(album_artists, target, ctx)),
                PlayGrouping::Albums => Child::Tags(TagBrowserPane::new(albums, target, ctx)),
                PlayGrouping::Folders => Child::Folders(DirectoriesPane::with_target(target, ctx)),
                PlayGrouping::Lists => Child::Lists(PlaylistsPane::with_target(target, ctx)),
            };
            if let Err(err) = with_child!(&mut child, b => b.before_show(ctx)) {
                log::error!(error:? = err; "Play's Browse could not load {grouping}");
            }
            child
        })
    }

    /// Show Browse on `grouping` with the keys.
    fn show_browse(&mut self, grouping: PlayGrouping, ctx: &Ctx) {
        self.left = Left::Browse;
        self.grouping = grouping;
        self.browse_focus = true;
        self.hits.set_focus_filters(false);
        self.ensure_child(grouping, ctx);
    }

    /// Back to the filters (they have the keys when `focus_filters`), Browse keeps its place.
    fn show_filters(&mut self, focus_filters: bool) {
        self.left = Left::Filters;
        self.browse_focus = false;
        self.hits.set_focus_filters(focus_filters);
    }

    /// `ShowPlay` (5-9, 0, gl) asked for a view.
    fn take_view_request(&mut self, ctx: &Ctx) {
        match rormpc_play::take_view() {
            Some(PlayView::Queue) => {
                self.overlay = Overlay::None;
                self.show_filters(false);
            }
            Some(PlayView::Browse(g)) => {
                self.overlay = Overlay::None;
                self.show_browse(g, ctx);
            }
            Some(PlayView::Live) => self.open_overlay(Overlay::Live, ctx),
            Some(PlayView::Deleted) => self.open_overlay(Overlay::Deleted, ctx),
            // gu: the table gets the keys, the cursor goes to its "Up next · N" row
            Some(PlayView::UpNext) => {
                self.overlay = Overlay::None;
                self.show_filters(false);
                if self.shows_hits_table() {
                    status_info!("Esc drops the preview first, then gu");
                } else if !self.queue.jump_to_up_next(ctx) {
                    status_info!("Up next is empty: t puts a song there");
                }
            }
            None => {}
        }
    }

    /// Open a panel over Play (it reloads), closing the other one.
    fn open_overlay(&mut self, overlay: Overlay, ctx: &Ctx) {
        self.overlay = overlay;
        let loaded = match overlay {
            Overlay::Live => self.live.before_show(ctx),
            Overlay::Deleted => self.deleted.before_show(ctx),
            Overlay::None => Ok(()),
        };
        if let Err(err) = loaded {
            log::error!(error:? = err; "{overlay:?} could not load");
        }
    }

    /// A badge click: open its panel, or close it when it is the open one.
    fn toggle_overlay(&mut self, overlay: Overlay, ctx: &Ctx) {
        if self.overlay == overlay {
            self.overlay = Overlay::None;
        } else {
            self.open_overlay(overlay, ctx);
        }
    }

    /// The stored playlist open in Lists (its editor panel is over Play), or None.
    fn editing(&self) -> Option<String> {
        if self.left != Left::Browse || self.grouping != PlayGrouping::Lists {
            return None;
        }
        match self.children.get(&PlayGrouping::Lists) {
            Some(Child::Lists(b)) => match b.stack().path().as_slice() {
                [name] => Some(name.clone()),
                _ => None,
            },
            _ => None,
        }
    }

    /// Browse (or the playlist editor over Play) has the keys.
    fn browse_has_keys(&self) -> bool {
        self.left == Left::Browse && (self.browse_focus || self.editing().is_some())
    }

    /// Why the playlists `P`, `D`, J/K or Ctrl-r would touch are read-only: (name, owner), else None.
    fn read_only_lists(&self) -> Option<(String, &'static str)> {
        let Some(Child::Lists(b)) = self.children.get(&PlayGrouping::Lists) else { return None };
        let live = self.live.playlist_names();
        let names: Vec<String> = match b.stack().path().as_slice() {
            [name] => vec![name.clone()],
            _ => b
                .items(false)
                .filter_map(|(_, i)| match i {
                    DirOrSong::Dir { name, .. } => Some(name.clone()),
                    DirOrSong::Song(_) => None,
                })
                .collect(),
        };
        names.into_iter().find_map(|n| rormpc_browse::generated(&n, &live).map(|why| (n, why)))
    }

    /// `D` in Lists: inside a playlist the songs leave it at once (an immediate MPD edit, as before); at the
    /// Lists level whole playlists go only after a confirmation.
    fn delete_in_lists(&mut self, ctx: &Ctx) {
        let Some(Child::Lists(b)) = self.children.get_mut(&PlayGrouping::Lists) else { return };
        let path = b.stack().path().as_slice().to_vec();
        let items = b.delete_items(false);
        if items.is_empty() {
            return;
        }
        b.stack_mut().current_mut().marked_mut().clear();
        if let [name] = path.as_slice() {
            let (n, name) = (items.len(), name.clone());
            ctx.command(move |_, client| {
                client.delete_multiple(items)?;
                status_info!("Removed {n} from \"{name}\" (saved at once)");
                Ok(())
            });
            return;
        }
        let names: Vec<String> = items
            .iter()
            .filter_map(|d| match d {
                MpdDelete::Playlist { name } => Some(format!("\"{name}\"")),
                MpdDelete::SongInPlaylist { .. } => None,
            })
            .collect();
        let message = vec![
            format!("Delete the playlist{} {}?", if names.len() == 1 { "" } else { "s" }, names.join(", ")),
            "MPD deletes it at once, for every client (phones too); this cannot be undone.".to_owned(),
        ];
        let go = move |ctx: &Ctx| -> Result<()> {
            let n = items.len();
            ctx.command(move |_, client| {
                client.delete_multiple(items)?;
                status_info!("Deleted {n} playlist{}", if n == 1 { "" } else { "s" });
                Ok(())
            });
            Ok(())
        };
        modal!(
            ctx,
            ConfirmModal::builder()
                .ctx(ctx)
                .message(message)
                .action(Action::CustomButtons { buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), ("Delete", Box::new(go))] })
                .build()
        );
    }

    /// A key in Browse (or the playlist editor): Play's own actions first, the rest goes to the grouping's
    /// browser.
    fn browse_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let grouping = self.grouping;
        let common = event.actions.iter().find_map(|a| a.as_common()).cloned();
        let editing = self.editing();
        let marks = {
            let child = self.ensure_child(grouping, ctx);
            with_child!(child, b => !b.stack().current().marked().is_empty())
        };
        match &common {
            // Esc: marks first (the browser clears them), then the editor panel, then Browse itself
            Some(CommonAction::Close) if !marks => {
                let _ = event.claim_common();
                if editing.is_some() {
                    if let Some(Child::Lists(b)) = self.children.get_mut(&PlayGrouping::Lists) {
                        b.stack_mut().leave();
                    }
                } else {
                    self.show_filters(false);
                }
                return Ok(ctx.render()?);
            }
            Some(CommonAction::PlayReplace) => {
                let _ = event.claim_common();
                let child = self.ensure_child(grouping, ctx);
                match with_child!(child, b => rormpc_browse::collection(b, grouping, ctx)) {
                    Ok(Some(col)) => rormpc_play::play_collection(ctx, col),
                    Ok(None) => status_info!("Nothing selected to play"),
                    Err(err) => status_error!("Cannot list the songs: {err}"),
                }
                return Ok(());
            }
            Some(CommonAction::Delete | CommonAction::MoveUp | CommonAction::MoveDown | CommonAction::Rename)
                if grouping == PlayGrouping::Lists =>
            {
                if let Some((name, why)) = self.read_only_lists() {
                    let _ = event.claim_common();
                    status_warn!("\"{name}\" is read-only here: {why}");
                    return Ok(());
                }
                if matches!(common, Some(CommonAction::Delete)) {
                    let _ = event.claim_common();
                    self.delete_in_lists(ctx);
                    return Ok(ctx.render()?);
                }
            }
            // a / A: append; a Hits source takes the appended songs into its files and round ("+N added")
            Some(CommonAction::AddOptions { kind: AddKind::Action(opts) })
                if opts.position == Position::EndOfQueue
                    && rormpc_upnext::source_info().is_some_and(|(kind, _, _)| kind == "hits") =>
            {
                let child = self.ensure_child(grouping, ctx);
                let listed = with_child!(child, b => ctx.query_sync(b.list_songs_in_items(opts.all)));
                let files: Vec<String> = match listed {
                    Ok(songs) => songs.into_iter().map(|s| s.file).collect(),
                    Err(err) => {
                        status_error!("Cannot list the songs: {err}");
                        Vec::new()
                    }
                };
                with_child!(child, b => b.handle_action(event, ctx))?;
                let joined = rormpc_upnext::note_appended(&files);
                if joined > 0 {
                    status_info!("Appended: {joined} joined the Hits source and its round (+{joined} added)");
                }
                return Ok(());
            }
            _ => {}
        }
        let child = self.ensure_child(grouping, ctx);
        with_child!(child, b => b.handle_action(event, ctx))
    }

    fn render_browse(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let [chips, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        self.render_chips(frame, chips, ctx);
        self.areas.browser = rest;
        if let Some(name) = self.editing() {
            // the editor is the panel over Play; the column only says so
            let dim = Style::default().add_modifier(Modifier::DIM);
            let text = format!(" Editing \"{name}\" in the panel · Esc closes it");
            frame.render_widget(Paragraph::new(Line::from(Span::styled(text, dim))), rest);
            return Ok(());
        }
        let grouping = self.grouping;
        let child = self.ensure_child(grouping, ctx);
        with_child!(child, b => b.render(frame, rest, ctx))
    }

    /// `Filters │ Browse`, the active side highlighted; each side is clickable.
    fn render_switch(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let on = ctx.config.theme.preview_label_style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        let off = Style::default().add_modifier(Modifier::DIM);
        let (f, b) = if self.left == Left::Filters { (on, off) } else { (off, on) };
        let line = Line::from(vec![Span::styled(" Filters ", f), Span::raw("│"), Span::styled(" Browse ", b), Span::styled(" B", off)]);
        frame.render_widget(Paragraph::new(line), area);
        self.areas.switch_filters = Rect { width: 9.min(area.width), ..area };
        self.areas.switch_browse =
            Rect { x: area.x + 10, width: 8.min(area.width.saturating_sub(10)), ..area };
    }

    /// The groupings as clickable chips, the shown one highlighted; short labels when the column is narrow.
    fn render_chips(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        use strum::VariantArray as _;
        let on = ctx.config.theme.preview_label_style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        let off = Style::default();
        let full: usize = PlayGrouping::VARIANTS.iter().map(|g| g.label().chars().count() + 3).sum();
        let short = full > usize::from(area.width);
        let mut spans = Vec::new();
        let mut x = area.x;
        self.areas.chips.clear();
        for g in PlayGrouping::VARIANTS {
            let label = if short { g.label().chars().take(3).collect::<String>() } else { g.label().to_owned() };
            let text = format!(" {label} ");
            let w = text.chars().count() as u16;
            self.areas.chips.push((*g, Rect { x, width: w.min((area.x + area.width).saturating_sub(x)), ..area }));
            spans.push(Span::styled(text, if *g == self.grouping { on } else { off }));
            spans.push(Span::raw(" "));
            x = x.saturating_add(w + 1);
        }
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    /// A panel over Play's body (the Live inbox, the playlist editor): cleared, bordered, titled.
    fn panel(frame: &mut Frame, area: Rect, title: String, ctx: &Ctx) -> (Rect, Rect) {
        let rect = Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height,
        };
        frame.render_widget(Clear, rect);
        if let Some(bg) = ctx.config.theme.modal_background_color {
            frame.render_widget(Block::default().style(Style::default().bg(bg)), rect);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(ctx.config.as_border_style())
            .title(title);
        let inner = block.inner(rect);
        frame.render_widget(block, rect);
        (rect, inner)
    }

    fn render_overlays(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        self.areas.overlay = Rect::default();
        match self.overlay {
            Overlay::Live => {
                let title = " Live playlists · Space marks · a accept · D reject · Enter menu · Esc closes ".to_owned();
                let (rect, inner) = Self::panel(frame, area, title, ctx);
                self.areas.overlay = rect;
                return self.live.render(frame, inner, ctx);
            }
            Overlay::Deleted => {
                let title = " Deleted · Enter restore, retry, allow or block downloading · Esc closes ".to_owned();
                let (rect, inner) = Self::panel(frame, area, title, ctx);
                self.areas.overlay = rect;
                return self.deleted.render(frame, inner, ctx);
            }
            Overlay::None => {}
        }
        if let Some(name) = self.editing() {
            let title = match rormpc_browse::generated(&name, &self.live.playlist_names()) {
                Some(why) => format!(" \"{name}\" is read-only: {why} · P play · t next · a append · Esc closes "),
                None => format!(
                    " Editing \"{name}\" · changes are saved at once · J/K move · D remove · P play · t next · a append · Esc closes "
                ),
            };
            let (rect, inner) = Self::panel(frame, area, title, ctx);
            self.areas.overlay = rect;
            if let Some(Child::Lists(b)) = self.children.get_mut(&PlayGrouping::Lists) {
                b.render(frame, inner, ctx)?;
            }
        }
        Ok(())
    }

    /// The Live badge: items waiting for a decision and running downloads ("Live 3 · ↓ 2"); always a click
    /// target that opens the inbox.
    fn badge(&self) -> String {
        let (pending, downloads) = self.live.badge();
        let mut text = " Live".to_owned();
        if pending > 0 {
            let _ = write!(text, " {pending}");
        }
        if downloads > 0 {
            let _ = write!(text, " · ↓ {downloads}");
        }
        text.push(' ');
        text
    }

    /// The Deleted badge ("Deleted ! 2"): only while deletions have failed or unresolved steps.
    fn deleted_badge(&self) -> Option<String> {
        let n = self.deleted.attention();
        (n > 0).then(|| format!(" Deleted ! {n} "))
    }

    // ---------------------------------------------------------------- Queue body

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
        let recomputed = std::mem::replace(&mut self.wait, ApplyWait::No) == ApplyWait::Recomputing;
        if let Some(err) = info.error {
            return status_error!("No preview to apply: {err}");
        }
        // a song deleted since the preview was made is not played from it: hits runs again first (once)
        if !recomputed {
            let changes = [rormpc_play::db_updated_at(ctx), self.deleted.newest_deletion()];
            if rormpc_play::preview_outdated(self.hits.result_mtime(), &changes) {
                self.wait = ApplyWait::Recomputing;
                self.hits.recompute(ctx);
                return status_info!("Preview outdated (the library changed since), recomputing: Apply follows");
            }
        }
        if info.files.is_empty() {
            return status_info!("0 owned songs: widen the filter (the queue stays as it is)");
        }
        let (Some(rules), Some(rules_hash)) = (self.hits.applied_rules(), self.hits.rules_hash()) else {
            return status_info!("The preview is still running: Apply when its counts are in the banner");
        };
        // a smart list played as saved is named after it ("Playing from: Hits · smart list 80s party")
        let name = match self.open_list() {
            Some((_, name, false)) => format!("smart list {name}"),
            _ => info.label,
        };
        rormpc_play::apply(ctx, HitsSource { name, files: info.files, rules, rules_hash });
    }

    /// Esc with nothing left to close inside the table: drop the preview, else close the column (normal mode).
    /// False when Play has nothing to do with it.
    fn escape(&mut self) -> bool {
        if self.preview_active() {
            self.hits.reset_filters(self.baseline.as_ref());
            self.wait = ApplyWait::No;
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
        // the open list, unless the source line already names it as played
        let list = match self.open_list() {
            Some((_, name, changed)) if changed || self.preview_active() => {
                format!(" · Smart list: {name}{}", if changed { " (changed)" } else { "" })
            }
            _ => String::new(),
        };
        // `a` appends in Browse and applies elsewhere: say where the keys are
        let browse = match (self.left, self.browse_has_keys()) {
            (Left::Browse, true) => format!(" · Browse › {} (a appends)", self.grouping.label()),
            (Left::Browse, false) => format!(" · Browse › {}", self.grouping.label()),
            (Left::Filters, _) => String::new(),
        };
        format!("▶ {source} · {mode}{list}{browse}")
    }

    /// The collapsed filter column: the rules of the source in one line.
    fn collapsed_line(&self) -> String {
        let rules = match rormpc_upnext::source_info().map(|(kind, name, _)| (kind, name)) {
            Some((kind, _)) if kind == "hits" && self.baseline.is_some() => {
                let formula = self.hits.formula().unwrap_or_default();
                match self.open_list() {
                    Some((_, name, _)) => format!("smart list {name} · {formula}"),
                    None => formula,
                }
            }
            Some((kind, name)) if kind == "hits" => format!("Hits · {name}"),
            Some((kind, _)) if kind == "library" => "whole library".to_owned(),
            Some((kind, name)) if matches!(kind.as_str(), "album" | "artist" | "directory" | "selection") => {
                format!("{} {name}", rormpc_upnext::kind_label(&kind))
            }
            Some((_, name)) => name,
            None => "none yet".to_owned(),
        };
        format!(" Source: {rules}  [h: filters · B: browse · L: lists · S: save]")
    }

    fn render_queue_body(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let preview = self.preview_active();
        let [banner, rest] =
            Layout::vertical([Constraint::Length(u16::from(preview)), Constraint::Min(1)]).areas(area);
        self.apply_area = Rect::default();
        if preview {
            let (text, enabled) = rormpc_play::banner(
                &self.hits.preview_info(),
                ctx.current_song().map(|s| s.file.as_str()),
                self.wait == ApplyWait::Recomputing,
            );
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
        self.areas.switch_filters = Rect::default();
        self.areas.switch_browse = Rect::default();
        self.areas.chips.clear();
        self.areas.browser = Rect::default();
        let (column, table) = if self.column_open() {
            // Browse needs room for its columns (the three-column browser); the filters keep the Hits width
            let width = if self.left == Left::Browse {
                Constraint::Length((rest.width / 2).max(40).min(rest.width.saturating_sub(30)))
            } else {
                Constraint::Length(COLUMN_WIDTH)
            };
            let [c, t] = Layout::horizontal([width, Constraint::Min(1)]).spacing(2).areas(rest);
            self.collapsed_area = Rect::default();
            let [switch, below] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(c);
            self.render_switch(frame, switch, ctx);
            (Some(below), t)
        } else {
            let [line, t] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(rest);
            self.collapsed_area = line;
            let dim = Style::default().add_modifier(Modifier::DIM);
            frame.render_widget(Paragraph::new(Line::from(Span::styled(self.collapsed_line(), dim))), line);
            (None, t)
        };
        let (filters, browse) = match (self.left, column) {
            (Left::Browse, Some(c)) => (None, Some(c)),
            (_, c) => (c, None),
        };
        if self.shows_hits_table() {
            let [dl_main, dl_details] =
                Layout::horizontal([Constraint::Min(40), Constraint::Percentage(40)]).spacing(2).areas(table);
            let body = HitsBody { main: table, details: None, dl_main, dl_details };
            self.hits.render_parts(frame, filters, Some(body), ctx);
            self.queue_area = Rect::default();
        } else {
            self.hits.render_parts(frame, filters, None, ctx);
            self.queue_area = table;
            self.queue.render(frame, table, ctx)?;
        }
        if let Some(b) = browse {
            self.render_browse(frame, b, ctx)?;
        }
        Ok(())
    }
}

impl Pane for PlayPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        self.take_view_request(ctx);
        // a pick first: `prepare` then runs `hits` for the rules it loaded in this same frame
        self.take_pick();
        self.hits.prepare(ctx);
        if self.hits.take_commit() {
            self.apply(ctx);
        }
        self.apply_if_ready(ctx);
        self.sync(ctx);
        // the badges follow the inbox and the journal while their panels are closed
        if self.overlay != Overlay::Live {
            self.live.refresh(ctx);
        }
        if self.overlay != Overlay::Deleted {
            self.deleted.refresh(ctx);
        }
        let [header, body] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let badge = self.badge();
        let deleted_badge = self.deleted_badge().unwrap_or_default();
        let [head, deleted_area, badge_area] = Layout::horizontal([
            Constraint::Min(1),
            Constraint::Length(deleted_badge.chars().count() as u16),
            Constraint::Length(badge.chars().count() as u16),
        ])
        .areas(header);
        let line = Line::from(Span::styled(self.header_line(ctx), ctx.config.theme.preview_label_style.add_modifier(Modifier::BOLD)));
        frame.render_widget(Paragraph::new(line), head);
        let lit = ctx.config.theme.preview_label_style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        let pending = self.live.badge() != (0, 0);
        let style = if pending { lit } else { Style::default().add_modifier(Modifier::DIM) };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(deleted_badge, lit))), deleted_area);
        frame.render_widget(Paragraph::new(Line::from(Span::styled(badge, style))), badge_area);
        self.areas.badge = badge_area;
        self.areas.deleted_badge = deleted_area;
        // mpd-player's rejected Up next command, even with no request left; it goes with its next explicit
        // successful action
        let error = rormpc_upnext::error();
        let [body, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(u16::from(error.is_some()))]).areas(body);
        if let Some(error) = error {
            let line = Line::from(Span::styled(format!(" Up next: {error}"), Style::default().fg(Color::Red)));
            frame.render_widget(Paragraph::new(line), footer);
        }
        self.render_queue_body(frame, body, ctx)?;
        self.render_overlays(frame, body, ctx)
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.sync(ctx);
        self.live.before_show(ctx)?; // the badges' counts
        self.deleted.before_show(ctx)?;
        self.hits.before_show(ctx)?;
        self.queue.before_show(ctx)
    }

    fn on_hide(&mut self, ctx: &Ctx) -> Result<()> {
        self.hits.on_hide(ctx)?;
        self.queue.on_hide(ctx)
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        self.hits.on_event(event, is_visible, ctx)?;
        for (grouping, child) in &mut self.children {
            let shown = is_visible && self.left == Left::Browse && self.grouping == *grouping;
            with_child!(child, b => b.on_event(event, shown, ctx))?;
        }
        self.live.on_event(event, is_visible && self.overlay == Overlay::Live, ctx)?;
        // the journal reloads on a Ctrl-x or Ctrl-y while Music is shown: the badge follows it
        self.deleted.on_event(event, is_visible, ctx)?;
        self.queue.on_event(event, is_visible, ctx)
    }

    fn on_target_query_finished(
        &mut self,
        target: &PaneType,
        id: &'static str,
        data: MpdQueryResult,
        is_visible: bool,
        ctx: &Ctx,
    ) -> Result<()> {
        // each grouping's reply goes to its own browser, shown or not (a switch before the reply is harmless)
        if let PaneType::PlayBrowse(grouping) = target {
            let shown = is_visible && self.left == Left::Browse && self.grouping == *grouping;
            return match self.children.get_mut(grouping) {
                Some(child) => with_child!(child, b => b.on_query_finished(id, data, shown, ctx)),
                None => Ok(()),
            };
        }
        self.on_query_finished(id, data, is_visible, ctx)
    }

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &mut Ctx) -> Result<()> {
        if self.browse_has_keys() {
            let grouping = self.grouping;
            let child = self.ensure_child(grouping, ctx);
            return with_child!(child, b => Pane::handle_insert_mode(b, kind, ctx));
        }
        if self.keys_to_hits() || self.hits.typing() {
            self.hits.handle_insert_mode(kind, ctx)
        } else {
            self.queue.handle_insert_mode(kind, ctx)
        }
    }

    fn handle_insert_nav(&mut self, down: bool, handled: &mut bool, ctx: &mut Ctx) -> Result<()> {
        if self.browse_has_keys() {
            return Ok(()); // the browsers' search has no Up/Down of its own
        }
        if self.keys_to_hits() || self.hits.typing() {
            self.hits.handle_insert_nav(down, handled, ctx)
        } else {
            self.queue.handle_insert_nav(down, handled, ctx)
        }
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        self.sync(ctx);
        self.take_view_request(ctx);
        let common = event.actions.iter().find_map(|a| a.as_common()).cloned();
        let queue_action = event.actions.iter().find_map(|a| a.as_queue()).cloned();
        // a panel over Play has every key; Esc (with nothing marked in the Live inbox) closes it
        match self.overlay {
            Overlay::Live | Overlay::Deleted
                if matches!(common, Some(CommonAction::Close))
                    && !(self.overlay == Overlay::Live && self.live.has_marks()) =>
            {
                let _ = event.claim_common();
                self.overlay = Overlay::None;
                return Ok(ctx.render()?);
            }
            Overlay::Live => return self.live.handle_action(event, ctx),
            Overlay::Deleted => return self.deleted.handle_action(event, ctx),
            Overlay::None => {}
        }
        // B, [ and ]: the left column and Browse's groupings, from anywhere in Play
        let typing = self.hits.typing();
        match queue_action {
            Some(QueueActions::ToggleBrowse) if !typing && event.claim_queue().is_some() => {
                if self.left == Left::Browse {
                    self.show_filters(true);
                } else {
                    self.show_browse(self.grouping, ctx);
                }
                return Ok(ctx.render()?);
            }
            Some(QueueActions::PreviousGrouping | QueueActions::NextGrouping)
                if self.left == Left::Browse && !typing && event.claim_queue().is_some() =>
            {
                let forward = matches!(queue_action, Some(QueueActions::NextGrouping));
                self.show_browse(self.grouping.step(forward), ctx);
                return Ok(ctx.render()?);
            }
            _ => {}
        }
        if self.browse_has_keys() {
            return self.browse_action(event, ctx);
        }
        // `a` is Apply everywhere else in Play (the row menu keeps "Add to queue")
        if matches!(common, Some(CommonAction::AddOptions { .. })) && event.claim_common().is_some() {
            self.apply(ctx);
            return Ok(ctx.render()?);
        }
        // S / L: smart lists, from the column, the preview or the queue alike (not while typing a search)
        if matches!(queue_action, Some(QueueActions::SaveSmartList | QueueActions::SmartLists))
            && !typing
            && event.claim_queue().is_some()
        {
            if matches!(queue_action, Some(QueueActions::SaveSmartList)) {
                self.save_list(ctx);
            } else {
                self.pick_list(ctx);
            }
            return Ok(());
        }
        if self.keys_to_hits() {
            self.hits.handle_action(event, ctx)?;
        } else {
            match (&common, &queue_action) {
                // h: the left column (the filters open in normal mode; Browse takes the keys back)
                (Some(CommonAction::Left), _) if event.claim_common().is_some() => {
                    if self.left == Left::Browse {
                        self.browse_focus = true;
                    } else {
                        self.hits.set_focus_filters(true);
                    }
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
                        let plus = self.baseline.as_ref().map_or_else(Vec::new, rormpc_exceptions::plus_sets_of_args);
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
        let at = event.into();
        // the badges open (or close) their panels
        for (area, overlay) in [(self.areas.badge, Overlay::Live), (self.areas.deleted_badge, Overlay::Deleted)] {
            if area.contains(at) {
                if click {
                    self.toggle_overlay(overlay, ctx);
                    ctx.render()?;
                }
                return Ok(());
            }
        }
        // a panel takes the mouse inside it; a click outside closes it
        if self.overlay != Overlay::None || self.editing().is_some() {
            if self.areas.overlay.contains(at) {
                match self.overlay {
                    Overlay::Live => return self.live.handle_mouse_event(event, ctx),
                    Overlay::Deleted => return self.deleted.handle_mouse_event(event, ctx),
                    Overlay::None => {}
                }
                if let Some(Child::Lists(b)) = self.children.get_mut(&PlayGrouping::Lists) {
                    return b.handle_mouse_event(event, ctx);
                }
                return Ok(());
            }
            if click {
                if self.overlay != Overlay::None {
                    self.overlay = Overlay::None;
                } else if let Some(Child::Lists(b)) = self.children.get_mut(&PlayGrouping::Lists) {
                    b.stack_mut().leave();
                }
                ctx.render()?;
            }
            return Ok(());
        }
        if self.areas.switch_filters.contains(at) || self.areas.switch_browse.contains(at) {
            if click {
                if self.areas.switch_filters.contains(at) {
                    self.show_filters(true);
                } else {
                    self.show_browse(self.grouping, ctx);
                }
                ctx.render()?;
            }
            return Ok(());
        }
        if let Some(grouping) = self.areas.chips.iter().find(|(_, r)| r.contains(at)).map(|(g, _)| *g) {
            if click {
                self.show_browse(grouping, ctx);
                ctx.render()?;
            }
            return Ok(());
        }
        if self.left == Left::Browse && self.areas.browser.contains(at) {
            if click && !self.browse_focus {
                self.browse_focus = true;
                ctx.render()?;
            }
            let grouping = self.grouping;
            let child = self.ensure_child(grouping, ctx);
            return with_child!(child, b => b.handle_mouse_event(event, ctx));
        }
        if self.apply_area.contains(at) {
            if click {
                self.apply(ctx);
                ctx.render()?;
            }
            return Ok(());
        }
        if self.collapsed_area.contains(at) {
            if click {
                self.hits.set_focus_filters(true);
                ctx.render()?;
            }
            return Ok(());
        }
        if self.queue_area.contains(at) {
            if click && (self.hits.focus_filters() || self.browse_focus) {
                self.hits.set_focus_filters(false);
                self.browse_focus = false;
                ctx.render()?;
            }
            return self.queue.handle_mouse_event(event, ctx);
        }
        if click && self.browse_focus && self.shows_hits_table() {
            self.browse_focus = false; // a click on the preview table gives it the keys
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

#[cfg(test)]
mod tests {
    use crate::config::tabs::PlayGrouping;

    #[test]
    fn groupings_cycle_both_ways() {
        assert_eq!(PlayGrouping::Artists.step(true), PlayGrouping::AlbumArtists);
        assert_eq!(PlayGrouping::Lists.step(true), PlayGrouping::Artists);
        assert_eq!(PlayGrouping::Artists.step(false), PlayGrouping::Lists);
        assert_eq!(PlayGrouping::Folders.step(false), PlayGrouping::Albums);
    }
}
