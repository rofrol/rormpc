//! rormpc: the Play pane (plans/combined-view.md, phases 3 and 3b): Queue, Hits, Shuffle and the browsing tabs
//! in one tab.
//!
//! - The left column (Filters | Browse) is always open, in both modes (decided by the user 2026-10-10); `h` and
//!   `l` move the keys between it and the table.
//! - Normal mode (weighted off): MPD's queue in its order.
//! - Weighted mode (`w`): the plan projection (past plays, `0 ▶`, Up next and the forecast; a pool song shows
//!   only as a `/` match). Turning weighted off keeps the queue as it is.
//! - Filters apply at once (plans/instant-filtering.md, decided by the user 2026-10-11): every filter change runs
//!   `hits`, bumps the generation, and its result, once in and still the newest, reconciles the queue
//!   (`rormpc_upnext::reconcile_hits_source`): only the songs the filters put there change, the playing song plays
//!   on, Up next stays. A queue holding another kind of source is replaced by the first filter touch. There is no
//!   preview and no Apply; the line under the header has the counts (`287 owned · 25 missing · was 312`), and `o`
//!   switches the table between the result rows and the plan (weighted) or the queue.
//! - `z` / `Z` (global `FilterUndo` / `FilterRedo`, from any tab) walk the filter history (`rormpc_undo`): an undo
//!   is a filter change itself, and past the first touch it brings the replaced source back in its order.
//! - The header line always says what plays and how: `▶ Playing from: … · weighted · round 12/84`, and the open
//!   smart list (`Smart list: 80s party`, "changed" once the filters differ from its rules).
//! - Smart lists (`rormpc_smartlists`): `S` saves the filters, `L` picks a list, a previous source or a playlist;
//!   a picked list's rules apply like any filter change.
//! - Phase 3b (decided by the user 2026-10-10): the left column switches Filters | Browse (`B`, or a click on
//!   the switch). Browse has the groupings Artists · Album artists · Albums · Folders · Lists (`[` `]`, the chip
//!   row, or 5-9 from anywhere); each is the browser pane of the old tab, owned here and created on first use, with
//!   its own query target (`PaneType::PlayBrowse`). In Browse `P` plays the selection replacing the queue (in its
//!   order; it asks first only on another source kind or more than a quarter of the queue going), `t` puts it
//!   into Up next, `a`/`A` append (a Hits source takes the appended songs into its round). A stored playlist opened in Lists is edited in a panel over Play (moves, removals and
//!   renames are saved at once; generated playlists are read-only), and the Live playlists inbox is a panel too
//!   (`0`, `gl`, or the `Live N` badge in the header).
//! - The Deleted pane is a panel over Music too (decided by the user 2026-10-10): `gd`, or the `Deleted ! N` badge,
//!   shown only while deletions have failed or unresolved steps.
//! - "Years to review" (`gY`, `rormpc_years`) is a panel over Music like Deleted.
//! - `d` on a queue song removes it and bans it (`rormpc_banned`, stage 5); the ban is an entry of the `z` / `Z`
//!   timeline, and the Banned panel (`gB`) lists the bans and unbans one (the filters rerun, so it comes back).
//! - Previous sources (`L`) records a rule set once a song of it has played, or when the filters' source is left
//!   for another one (`rormpc_smartlists::remember`), not on every applied change; an open smart list exports its
//!   playlist on the same rule.
//!
//! It is composed, not copied: the filter column and the result rows are a `HitsPane` in Play mode, the table
//! is a `QueuePane` whose view follows the weighted shuffle, Browse's groupings are the browser panes.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    sync::{Arc, atomic::Ordering},
};

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
    banned::BannedPane,
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
        rormpc_play::{self, LastChange, Step},
        rormpc_player,
        rormpc_smartlists::{self, Pick},
        rormpc_banned,
        rormpc_undo::{Ban, Departed, Entry, History, State},
        rormpc_upnext::{self, Generations, HitsSource},
        rormpc_years::YearsPane,
    },
};

/// Width of the filter column, as in the Hits pane.
const COLUMN_WIDTH: u16 = 24;

/// What Play's left column shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Left {
    /// the Hits filter column
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
    Banned,
    Years,
}

/// What the table right of the column shows (`o` switches it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Table {
    /// the plan projection (weighted) or MPD's queue
    Queue,
    /// the `hits` result rows, missing songs dimmed
    Rows,
}

/// The song Previous sources saw playing last.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Playing {
    /// not looked yet: the song playing when rormpc started is no start
    NotLooked,
    /// this song id (None: nothing plays)
    Song(Option<u32>),
}

/// An undo or redo whose rules' `hits` run has not reached the queue yet.
#[derive(Debug)]
struct Restoring {
    /// the rules' hash: a result with other rules is a newer change of the user's
    hash: String,
    /// what the counts line says once it is in ("Undo: rock cleared")
    label: String,
}

/// What a change from `before` to `rules` did, in a few words.
fn describe(before: Option<&State>, rules: &serde_json::Value) -> String {
    match before {
        Some(State::Rules(old)) => HitsPane::describe_change(old, rules),
        Some(State::Other(d)) => format!("replaced {}", departed_label(d)),
        None => "applied".to_owned(),
    }
}

/// "album Low", "playlist 80s", "the whole library".
fn departed_label(d: &Departed) -> String {
    match d.kind.as_str() {
        "library" => "the whole library".to_owned(),
        "selection" => d.name.clone(),
        kind => format!("{} {}", rormpc_upnext::kind_label(kind), d.name),
    }
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
    /// the songs a hand removal banned (`gB`)
    banned: BannedPane,
    /// the release years to review (`gY`)
    years: YearsPane,
    overlay: Overlay,
    /// the rules the source being played was applied with (source.json), None for any other source: the
    /// filters start from them
    baseline: Option<serde_json::Value>,
    /// source.json's rules hash when `baseline` was taken (a reconcile or another source moves it); `synced`
    /// once taken
    source_hash: Option<String>,
    synced: bool,
    /// the newest filter intent (every `hits` run the filters start bumps it, and a reconcile still queued for
    /// an older one is dropped on the MPD command thread) and the newest one whose writes started
    gens: Generations,
    /// `z` / `Z`: the filter states the changes that reached MPD replaced (`rormpc_undo`)
    history: History,
    /// what the last change did ("Filter: +rock", "Undo: rock cleared"), for the counts line
    last_change: Option<String>,
    /// an undo or redo under way: its rules' run is no new history entry
    restoring: Option<Restoring>,
    /// an undo bringing another source back (kind, name): source.json naming it is no outside change
    expect_other: Option<(String, String)>,
    /// rules hashes this pane reconciled the queue with: source.json naming one of them is ours, so it never
    /// resets the filters (a newer change may be on screen already)
    sent: HashSet<String>,
    /// owned songs of the last two generations sent to MPD (the counts line's "was")
    owned: Option<usize>,
    owned_was: Option<usize>,
    /// `o`: the table shows the `hits` result rows instead of the plan or the queue
    table: Table,
    /// `a` in the Queue body said once that filters apply at once
    a_hint_said: bool,
    /// Previous sources: the song seen playing last, and the rules hash remembered for the rule set in the queue
    playing_seen: Playing,
    remembered: Option<String>,
    /// what the list picker and the Save modal hand back (`rormpc_smartlists::Pick`)
    inbox: rormpc_smartlists::Inbox,
    /// areas of the last render, for the mouse
    toggle_area: Rect,
    queue_area: Rect,
    areas: BrowseAreas,
}

impl PlayPane {
    pub fn new(ctx: &Ctx) -> Self {
        let gens = Generations::default();
        let mut s = Self {
            hits: HitsPane::for_play(rormpc_play::preview_file(), vec!["hits".to_owned()], Arc::clone(&gens.latest)),
            queue: QueuePane::new(ctx),
            left: Left::Filters,
            browse_focus: false,
            grouping: PlayGrouping::Albums,
            children: HashMap::new(),
            live: LivePlaylistsPane::new(),
            deleted: DeletedPane::new(),
            banned: BannedPane::new(),
            years: YearsPane::new(),
            overlay: Overlay::None,
            baseline: None,
            source_hash: None,
            synced: false,
            gens,
            history: History::default(),
            last_change: None,
            restoring: None,
            expect_other: None,
            sent: HashSet::new(),
            owned: None,
            owned_was: None,
            table: Table::Queue,
            a_hint_said: false,
            playing_seen: Playing::NotLooked,
            remembered: None,
            inbox: rormpc_smartlists::Inbox::default(),
            toggle_area: Rect::default(),
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

    /// Take what the list picker or the Save modal left: load rules (they apply like any filter change), or
    /// leave the open list.
    fn take_pick(&mut self) {
        let pick = self.inbox.lock().ok().and_then(|mut p| p.take());
        let args = match pick {
            Some(Pick::Load(args)) => args,
            Some(Pick::CloseList) => {
                self.hits.close_list();
                serde_json::Value::Null
            }
            None => serde_json::Value::Null,
        };
        if !args.is_null() {
            let same = Some(HitsPane::rules_hash_of(Some(&args))) == self.source_hash;
            if let Err(err) = self.hits.load_filters(&args) {
                status_error!("These rules cannot be loaded: {err}");
            } else if same {
                status_info!("The queue already plays these rules");
            }
        }
        rormpc_smartlists::set_open(self.open_list().map(|(id, name, _)| (id, name)));
    }

    /// A newest `hits` result came in and the filters on screen made it: the queue follows it (generation
    /// tagged, so a newer change still under way drops it on the MPD command thread).
    fn apply_result(&mut self, ctx: &Ctx) {
        if !self.hits.take_fresh() {
            return;
        }
        let info = self.hits.preview_info();
        if !info.ready {
            return; // the filters moved on: their own run follows
        }
        let (Some(rules), Some(rules_hash)) = (self.hits.applied_rules(), self.hits.rules_hash()) else { return };
        // a smart list played as saved is named after it ("Playing from: Hits · smart list 80s party")
        let name = match self.open_list() {
            Some((_, name, false)) => format!("smart list {name}"),
            _ => info.label,
        };
        self.owned_was = self.owned;
        self.owned = Some(info.files.len());
        self.sent.insert(rules_hash.clone());
        let generation = self.gens.latest.load(Ordering::SeqCst);
        // the history: an undo's or redo's own run is no entry; a user's change records what it replaced, the
        // other kind of source in the queue on the first touch
        let restore = self.restoring.take().filter(|r| r.hash == rules_hash);
        if !rormpc_upnext::holds_filter_source() {
            self.history.set_in_mpd(State::Other(rormpc_upnext::departed(ctx)));
        }
        self.last_change = Some(match &restore {
            Some(r) => r.label.clone(),
            // the same rules again: an exception or the library changed their result
            None => match describe(self.history.current(), &rules) {
                same if same == "same rules" => "Result updated (same rules)".to_owned(),
                what => format!("Filter: {what}"),
            },
        });
        let reached = self.gens.reached.load(Ordering::SeqCst);
        self.history.sent(generation, State::Rules(rules.clone()), restore.is_none(), reached);
        let in_order = !rormpc_player::shuffle_state().enabled;
        let src = HitsSource { name, files: info.files, rules, rules_hash };
        rormpc_upnext::reconcile_hits_source(ctx, src, generation, self.gens.clone(), in_order);
    }

    /// `z` / `Z` asked for since the last frame, from any tab, in order, and the bans hand removals made (they
    /// share the timeline); then the songs the Banned panel unbanned, which the filters take back in.
    fn take_steps(&mut self, ctx: &Ctx) {
        for step in rormpc_play::take_steps() {
            match step {
                Step::Banned(ban) => self.history.banned(ban),
                step => self.step(&step, ctx),
            }
        }
        let unbanned = self.banned.take_unbanned();
        if !unbanned.is_empty() {
            let label = format!("Unbanned {}", unbanned.len());
            self.last_change = Some(label.clone());
            self.rerun_filters(label);
        }
    }

    /// Run `hits` again for the filters on screen (a ban or unban changed what they match): its result
    /// reconciles the queue as no new history entry, the counts line saying `label`. Only while the queue holds
    /// the filters' source: an album or a playlist is not replaced behind the user's back.
    fn rerun_filters(&mut self, label: String) {
        if !rormpc_upnext::holds_filter_source() {
            return;
        }
        if let Some(hash) = self.hits.rules_hash() {
            self.restoring = Some(Restoring { hash, label });
            self.hits.request_rerun();
        }
    }

    /// `z` / `Z` on a ban: unban its songs (or ban them again), then rerun the filters so the queue follows.
    fn step_ban(&mut self, ban: Ban, undo: bool, ctx: &Ctx) {
        let label = format!("{}: {} {}", if undo { "Undo" } else { "Redo" }, if undo { "unbanned" } else { "banned" }, ban.what);
        self.last_change = Some(label.clone());
        let rerun = rormpc_upnext::holds_filter_source().then(|| self.hits.rerun_done(ctx));
        if let (Some(hash), true) = (self.hits.rules_hash(), rerun.is_some()) {
            self.restoring = Some(Restoring { hash, label });
        }
        rormpc_banned::set_banned(ctx, ban.files, !undo, move || {
            if let Some(rerun) = rerun {
                rerun();
            }
        });
    }

    /// One undo or redo: itself a filter change (it bumps the generation and the queue follows), restoring the
    /// membership, never the playback time. Undo while a change is still under way takes that change back.
    fn step(&mut self, step: &Step, ctx: &Ctx) {
        let running = self.hits.preview_info().running;
        if running && *step == Step::Redo {
            return status_info!("A filter change is under way: nothing to redo");
        }
        if running {
            self.hits.cancel_run();
            match self.history.current().cloned() {
                Some(State::Rules(r)) => self.restore_rules(&r, "Undo: the change under way".to_owned()),
                // the first touch was still running: the other source never left the queue
                _ if !rormpc_upnext::holds_filter_source() => {
                    self.gens.latest.fetch_add(1, Ordering::SeqCst);
                    self.hits.reset_filters(self.baseline.as_ref());
                    self.last_change = Some("Undo: the change under way".to_owned());
                }
                _ => {}
            }
            return;
        }
        let before = self.history.current().cloned();
        let (target, verb) = match step {
            Step::Undo => (self.history.undo(), "Undo"),
            Step::Redo => (self.history.redo(), "Redo"),
            Step::Banned(_) => return,
        };
        match target {
            None if *step == Step::Undo => status_info!("Nothing to undo in Music's filters or bans"),
            None => status_info!("Nothing to redo in Music's filters or bans"),
            Some(Entry::Banned(ban)) => self.step_ban(ban, *step == Step::Undo, ctx),
            Some(Entry::State(State::Rules(r))) => {
                let label = format!("{verb}: {}", describe(before.as_ref(), &r));
                self.restore_rules(&r, label);
            }
            Some(Entry::State(State::Other(d))) => {
                self.hits.cancel_run();
                let generation = self.gens.latest.fetch_add(1, Ordering::SeqCst) + 1;
                let label = format!("{verb}: back to {}", departed_label(&d));
                self.last_change = Some(label);
                self.restoring = None;
                self.expect_other = Some((d.kind.clone(), d.name.clone()));
                rormpc_upnext::restore_departed(ctx, d, generation, self.gens.clone());
            }
        }
    }

    /// Load `rules` into the filters: the next frame runs `hits` for them and the queue follows, as no new entry.
    fn restore_rules(&mut self, rules: &serde_json::Value, label: String) {
        if let Err(err) = self.hits.load_filters(rules) {
            return status_error!("These rules cannot be loaded: {err}");
        }
        self.restoring = Some(Restoring { hash: HitsPane::rules_hash_of(Some(rules)), label });
        rormpc_smartlists::set_open(self.open_list().map(|(id, name, _)| (id, name)));
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

    /// Follow what changed outside: the weighted shuffle switches the table's view, and a source this pane did
    /// not reconcile (Sources…, Browse's `P`, another rormpc) becomes the filters' starting point.
    fn sync(&mut self, ctx: &Ctx) {
        self.queue.follow_weighted(rormpc_player::shuffle_state().enabled, ctx);
        self.note_played(ctx);
        let rules = rormpc_upnext::source_rules();
        let hash = rules.as_ref().map(|(_, h)| h.clone());
        if self.synced && self.source_hash == hash {
            return;
        }
        let ours = self.synced && hash.as_ref().is_some_and(|h| self.sent.contains(h));
        self.synced = true;
        self.source_hash = hash;
        self.baseline = rules.map(|(r, _)| r);
        if !ours {
            self.hits.reset_filters(self.baseline.as_ref());
            self.sent.clear();
            self.owned = None;
            self.owned_was = None;
            // an undo brought this source back: the history goes on; anything else starts a new one
            let restored = self.expect_other.take().is_some_and(|(kind, name)| {
                rormpc_upnext::source_info().is_some_and(|(k, n, _)| k == kind && n == name)
            });
            if !restored {
                self.history.reset(self.baseline.clone().map(State::Rules));
                self.last_change = None;
                self.restoring = None;
            }
        }
    }

    /// Previous sources records a rule set once a song of it has played (decided by the user 2026-10-11): a song
    /// of the filter source started, so its rules are remembered, once per rule set.
    fn note_played(&mut self, ctx: &Ctx) {
        let song = ctx.current_song();
        let id = song.map(|s| s.id);
        let seen = std::mem::replace(&mut self.playing_seen, Playing::Song(id));
        if seen == Playing::NotLooked || seen == Playing::Song(id) {
            return;
        }
        let played = song.and_then(|s| rormpc_upnext::played_rules(&s.file));
        if let Some((name, rules, hash)) = played.filter(|(_, _, h)| self.remembered.as_ref() != Some(h)) {
            rormpc_smartlists::remember(&name, &rules, &hash);
            self.remembered = Some(hash);
        }
    }

    /// The table right of the column is the Hits result (`o`, or the Downloads view), not the queue.
    fn shows_hits_table(&self) -> bool {
        self.table == Table::Rows || self.hits.in_downloads()
    }

    /// Keys go to the Hits part (the column, or its table) rather than the queue.
    fn keys_to_hits(&self) -> bool {
        (self.left == Left::Filters && self.hits.focus_filters()) || self.shows_hits_table()
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
            Some(PlayView::Banned) => self.open_overlay(Overlay::Banned, ctx),
            Some(PlayView::Years) => self.open_overlay(Overlay::Years, ctx),
            // gu: the table gets the keys, the cursor goes to its "Up next · N" row
            Some(PlayView::UpNext) => {
                self.overlay = Overlay::None;
                self.show_filters(false);
                if self.hits.in_downloads() {
                    status_info!("Leave Downloads first, then gu");
                } else {
                    self.table = Table::Queue;
                    if !self.queue.jump_to_up_next(ctx) {
                        status_info!("Up next is empty: t puts a song there");
                    }
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
            Overlay::Banned => self.banned.before_show(ctx),
            Overlay::Years => self.years.before_show(ctx),
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
            Overlay::Banned => {
                let title = " Banned · Enter or u unbans · Esc closes ".to_owned();
                let (rect, inner) = Self::panel(frame, area, title, ctx);
                self.areas.overlay = rect;
                return self.banned.render(frame, inner, ctx);
            }
            Overlay::Years => {
                let title = " Years to review · Space marks · a accept · D reject (again: undecide) · h/l filter · Enter menu · Esc closes ".to_owned();
                let (rect, inner) = Self::panel(frame, area, title, ctx);
                self.areas.overlay = rect;
                return self.years.render(frame, inner, ctx);
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

    /// `a` in the Queue body: filters apply at once, so it only says so, once.
    fn a_hint(&mut self) {
        if !std::mem::replace(&mut self.a_hint_said, true) {
            status_info!("Filters apply at once; z undoes");
        }
    }

    /// Esc with nothing left to close inside the table: stop a running `hits` (its result never reaches the
    /// queue), else leave the result rows (`o`). False when Play has nothing to do with it.
    fn escape(&mut self) -> bool {
        if self.hits.stop_run() {
            return true;
        }
        if self.table == Table::Rows && !self.hits.in_downloads() {
            self.table = Table::Queue;
            return true;
        }
        false
    }

    /// `o`: the result rows ⇄ the plan (weighted) or the queue.
    fn toggle_rows(&mut self) {
        self.table = if self.table == Table::Rows { Table::Queue } else { Table::Rows };
        if self.table == Table::Rows {
            self.browse_focus = false;
            self.hits.set_focus_filters(false);
        }
    }

    fn header_line(&self, ctx: &Ctx) -> String {
        let source = rormpc_upnext::header(ctx).map_or_else(|| "Nothing chosen yet".to_owned(), |h| h.trim().to_owned());
        let shuffle = rormpc_player::shuffle_state();
        let mode = if shuffle.enabled {
            let round = shuffle.round.as_ref().map_or(String::new(), |r| {
                if r.done { " · round done".to_owned() } else { format!(" · round {}/{}", r.heard.len(), r.total) }
            });
            let waiting = if shuffle.active { String::new() } else { format!(" · waiting: {}", shuffle.reason) };
            let short = if shuffle.active && !shuffle.ahead_note.is_empty() {
                format!(" · {}", shuffle.ahead_note)
            } else {
                String::new()
            };
            format!("weighted{round}{short}{waiting}")
        } else {
            "in queue order · w: weighted".to_owned()
        };
        // the open list, unless the source line already names it as played
        let list = match self.open_list() {
            Some((_, name, changed)) if changed => {
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

    fn render_queue_body(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        // the counts line: what the filters made of the queue, and the rows ⇄ plan switch
        let [counts, rest] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
        let weighted = rormpc_player::shuffle_state().enabled;
        let other = if weighted { "plan" } else { "queue" };
        let toggle = format!("[ o rows ⇄ {other} ]");
        let width = toggle.chars().count() as u16;
        let [t, b] = Layout::horizontal([Constraint::Min(1), Constraint::Length(width)]).spacing(1).areas(counts);
        let asked = self.gens.latest.load(Ordering::SeqCst) > 0;
        let last = LastChange {
            what: self.last_change.as_deref(),
            undo: self.history.can_undo(),
            redo: self.history.can_redo(),
        };
        let text = rormpc_play::counts_line(&self.hits.preview_info(), self.owned_was, asked, last);
        frame.render_widget(Paragraph::new(Line::from(Span::styled(text, ctx.config.theme.preview_label_style))), t);
        let on = ctx.config.theme.preview_label_style.add_modifier(Modifier::REVERSED | Modifier::BOLD);
        let off = Style::default().add_modifier(Modifier::DIM);
        let (rows_style, other_style) = if self.table == Table::Rows { (on, off) } else { (off, on) };
        let line = Line::from(vec![
            Span::styled("[ o ", off),
            Span::styled("rows", rows_style),
            Span::styled(" ⇄ ", off),
            Span::styled(other, other_style),
            Span::styled(" ]", off),
        ]);
        frame.render_widget(Paragraph::new(line), b);
        self.toggle_area = b;
        self.areas.switch_filters = Rect::default();
        self.areas.switch_browse = Rect::default();
        self.areas.chips.clear();
        self.areas.browser = Rect::default();
        // Browse needs room for its columns (the three-column browser); the filters keep the Hits width
        let width = if self.left == Left::Browse {
            Constraint::Length((rest.width / 2).max(40).min(rest.width.saturating_sub(30)))
        } else {
            Constraint::Length(COLUMN_WIDTH)
        };
        let [column, table] = Layout::horizontal([width, Constraint::Min(1)]).spacing(2).areas(rest);
        let [switch, column] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(column);
        self.render_switch(frame, switch, ctx);
        let (filters, browse) = match self.left {
            Left::Browse => (None, Some(column)),
            Left::Filters => (Some(column), None),
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
        self.take_steps(ctx);
        self.hits.prepare(ctx);
        self.apply_result(ctx);
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
        self.banned.on_event(event, is_visible && self.overlay == Overlay::Banned, ctx)?;
        self.years.on_event(event, is_visible && self.overlay == Overlay::Years, ctx)?;
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
            Overlay::Live | Overlay::Deleted | Overlay::Banned | Overlay::Years
                if matches!(common, Some(CommonAction::Close))
                    && !(self.overlay == Overlay::Live && self.live.has_marks())
                    && !(self.overlay == Overlay::Years && self.years.has_marks()) =>
            {
                let _ = event.claim_common();
                self.overlay = Overlay::None;
                return Ok(ctx.render()?);
            }
            Overlay::Live => return self.live.handle_action(event, ctx),
            Overlay::Deleted => return self.deleted.handle_action(event, ctx),
            Overlay::Banned => return self.banned.handle_action(event, ctx),
            Overlay::Years => return self.years.handle_action(event, ctx),
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
        // `a` outside Browse: filters apply at once, it only says so (the row menu keeps "Add to queue")
        if matches!(common, Some(CommonAction::AddOptions { .. })) && !typing && event.claim_common().is_some() {
            self.a_hint();
            return Ok(ctx.render()?);
        }
        // o: the result rows ⇄ the plan or the queue
        if matches!(queue_action, Some(QueueActions::TogglePlanView)) && !typing && event.claim_queue().is_some() {
            self.toggle_rows();
            return Ok(ctx.render()?);
        }
        // S / L: smart lists, from the column, the result rows or the queue alike (not while typing a search)
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
                // h: the left column (the filters or Browse take the keys)
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
                // + / -: the scopes offered are the + sets of the rules being played; the exception applies at
                // once (the filters rerun) while the queue holds their source
                (_, Some(QueueActions::PinSong | QueueActions::ExcludeSong)) if event.claim_queue().is_some() => {
                    let kind = if matches!(queue_action, Some(QueueActions::PinSong)) { Kind::Pin } else { Kind::Exclude };
                    if let Some(song) = self.queue.selected_song(ctx) {
                        let plus = self.baseline.as_ref().map_or_else(Vec::new, rormpc_exceptions::plus_sets_of_args);
                        let rerun = self.hits.rerun_done(ctx);
                        let done: rormpc_exceptions::Done = Arc::new(move || {
                            if rormpc_upnext::holds_filter_source() {
                                rerun();
                            }
                        });
                        rormpc_exceptions::open_for_song_with_sets(ctx, kind, &song, plus, Some(done));
                    } else {
                        status_error!("No song selected");
                    }
                    return Ok(());
                }
                _ => self.queue.handle_action(event, ctx)?,
            }
        }
        // Esc the Hits part left alone: stop a run, or leave the result rows
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
                    Overlay::Banned => return self.banned.handle_mouse_event(event, ctx),
                    Overlay::Years => return self.years.handle_mouse_event(event, ctx),
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
        if self.toggle_area.contains(at) {
            if click {
                self.toggle_rows();
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
            self.browse_focus = false; // a click on the result rows gives them the keys
        }
        self.hits.handle_mouse_event(event, ctx)

    }

    fn resize(&mut self, area: Rect, ctx: &Ctx) -> Result<()> {
        self.queue.resize(area, ctx)
    }

    /// Music in no shown tab: `z` / `Z` still go through the filters, and their results reach the queue.
    fn background(&mut self, ctx: &Ctx) -> Result<()> {
        self.take_steps(ctx);
        self.hits.prepare(ctx);
        self.apply_result(ctx);
        self.sync(ctx);
        Ok(())
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
