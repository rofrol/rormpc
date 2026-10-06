//! rormpc: Hits pane. A table of the ranked chart hits written by the `hits` CLI (`hits ... --json PATH`) with
//! a details panel for the selected row. Rows are chart entries, not directories: a missing song is a dimmed
//! row with nothing to play. Enter / double click play the selected owned song (its queue entry if it is
//! already queued, else appended), `a` appends without playing unless already queued; the queue is never
//! replaced. The ranking itself stays in `hits`: the filter
//! column on the left (h/l moves between it and the table) runs `hits --json` in a background thread on Apply.
//! Missing songs can be fetched through `hits fetch` (a verified import queue): the menu queues them and starts
//! the worker, and each missing row shows its state from the queue file.

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
    config::keys::{CommonAction, QueueActions},
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
        events::AppEvent,
    },
    shared::macros::{modal, status_error, status_info},
    ui::{
        UiEvent,
        dirstack::DirState,
        input::{BufferId, InputResultEvent},
        modals::{
            confirm_modal::{Action, ConfirmModal},
            input_modal::InputModal,
            menu::modal::MenuModal,
        },
        rormpc_actions,
    },
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
    /// the artists of the whole cohort (before the Top % cut), for the artist picker
    #[serde(default)]
    artists: Vec<ArtistCount>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArtistCount {
    name: String,
    songs: u32,
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
    artist: Option<String>,
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
    /// why a recommendation is there ("similar to …")
    #[serde(default)]
    reason: Option<String>,
}

/// One genre of `hits genres --json` (the genre explorer).
#[derive(Debug, Clone, Deserialize)]
struct GenreInfo {
    name: String,
    songs: u32,
    recording: u32,
    #[serde(default)]
    plays: u32,
    #[serde(default)]
    pinned: bool,
}

#[derive(Debug, Deserialize)]
struct GenresFile {
    total: u32,
    unknown: u32,
    genres: Vec<GenreInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExplorerPick {
    Filter,
    FilterLikes,
    Pin,
    Unpin,
}

/// State shared with the thread that counts genres and the explorer menus.
#[derive(Debug, Default)]
struct Explorer {
    loading: bool,
    ready: Option<GenresFile>,
    pick: Option<(String, ExplorerPick)>,
}

/// Genres pinned as checkboxes (`hits genres pin`), else the built-in list.
fn pinned_genres() -> Vec<String> {
    crate::ui::rormpc_genres::pins().unwrap_or_else(|| GENRES.iter().map(|g| (*g).to_owned()).collect())
}

/// One song in the `hits fetch` queue ($XDG_STATE_HOME/rormpc-tools/fetch/queue.json).
#[derive(Debug, Clone, Deserialize)]
struct FetchItem {
    key: String,
    artist: String,
    title: String,
    #[serde(default)]
    mbid: Option<String>,
    state: String,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    candidate: Option<String>,
    /// what MusicBrainz identified the download as
    #[serde(default)]
    found: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FetchQueue {
    items: Vec<FetchItem>,
}

fn fetch_queue_path() -> PathBuf {
    let state = std::env::var("XDG_STATE_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(expand_home("~/.local/state"))
    });
    state.join("rormpc-tools/fetch/queue.json")
}

/// One-cell mark of a missing row's fetch state.
fn fetch_mark(state: &str) -> &'static str {
    match state {
        "queued" => "…",
        "searching" | "downloading" | "verifying" => "↓",
        "review" => "?",
        "failed" => "!",
        "ok" => "+",
        _ => "✗",
    }
}

#[derive(Debug)]
pub struct HitsPane {
    path: PathBuf,
    /// the rows shown: the result, narrowed by the `/` search
    rows: Vec<HitsRow>,
    /// the whole result of the last `hits` run
    all_rows: Vec<HitsRow>,
    /// `/` search over artist and title (client-side, never changes ranks)
    search: BufferId,
    typing: bool,
    query: String,
    /// the row whose like cell the mouse is over (shows a heart to click)
    hover_like: Option<usize>,
    /// the filter row under the mouse pointer: only a look (underline), never the cursor
    hover_filter: Option<usize>,
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
    /// the scrolled filter rows; Apply is drawn below it in `apply_area` and never scrolls away
    filter_area: Rect,
    apply_area: Rect,
    /// `hits` arguments of the result on screen, to show whether the filters changed since
    applied_args: Option<Vec<String>>,
    /// text typed into the "other genre" input, picked up on the next render
    genre_input: Arc<Mutex<Option<String>>>,
    /// artists of the last result's cohort, and the one picked in "+ artist…", taken on the next render
    cohort_artists: Vec<ArtistCount>,
    artist_input: Arc<Mutex<Option<String>>>,
    explorer: Arc<Mutex<Explorer>>,
    fetch: Vec<FetchItem>,
    fetch_mtime: Option<SystemTime>,
    /// the pins file as last read: a genre pinned elsewhere (Queue's "Pin genre…") gets its checkbox on the next render
    pins_mtime: Option<SystemTime>,
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
            apply_area: Rect::default(),
            applied_args: None,
            genre_input: Arc::new(Mutex::new(None)),
            cohort_artists: Vec::new(),
            artist_input: Arc::new(Mutex::new(None)),
            explorer: Arc::new(Mutex::new(Explorer::default())),
            fetch: Vec::new(),
            fetch_mtime: None,
            pins_mtime: None,
            job: Arc::new(Mutex::new(Job::default())),
            path: PathBuf::from(expand_home(&path)),
            rows: Vec::new(),
            all_rows: Vec::new(),
            search: BufferId::new(),
            typing: false,
            query: String::new(),
            hover_like: None,
            hover_filter: None,
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
                    let filters = Filters::from_args(&file.args);
                    self.applied_args = Some(filters.args(&self.path.to_string_lossy()));
                    self.filters = Some(filters);
                }
                let keep = self.selected().map(|r| r.rank);
                self.all_rows = file.rows;
                self.cohort_artists = file.artists;
                self.label = file.label;
                self.generated_at = file.generated_at;
                self.rank_note = file.rank_note.unwrap_or_default();
                self.error = None;
                self.refilter(keep);
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

    /// Re-read the fetch queue when the worker or a menu action changed it.
    fn reload_fetch(&mut self) {
        let path = fetch_queue_path();
        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if mtime == self.fetch_mtime {
            return;
        }
        self.fetch_mtime = mtime;
        self.fetch = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<FetchQueue>(&text).ok())
            .map(|q| q.items)
            .unwrap_or_default();
    }

    /// Give newly pinned genres a checkbox; unpinning leaves a box until the next start, as in the explorer.
    fn reload_pins(&mut self) {
        let mtime = std::fs::metadata(crate::ui::rormpc_genres::pins_path()).and_then(|m| m.modified()).ok();
        if mtime == self.pins_mtime {
            return;
        }
        self.pins_mtime = mtime;
        if let (Some(filters), Some(pins)) = (self.filters.as_mut(), crate::ui::rormpc_genres::pins()) {
            for genre in pins {
                if !filters.genres.iter().any(|(g, _)| *g == genre) {
                    filters.genres.push((genre, 0));
                }
            }
        }
    }

    /// The queue entry of a chart row: same recording MBID, else the same artist and title.
    fn fetch_for(&self, r: &HitsRow) -> Option<&FetchItem> {
        self.fetch.iter().find(|f| match (&f.mbid, &r.mbid) {
            (Some(a), Some(b)) => a == b,
            _ => f.artist == r.artist && f.title == r.title,
        })
    }

    /// Menu section for a missing row: fetch it (or more), or decide on what the queue found.
    fn fetch_section<'m>(&self, ctx: &Ctx, menu: MenuModal<'m>, r: &HitsRow) -> MenuModal<'m> {
        let path = self.path.to_string_lossy().into_owned();
        let missing = self.rows.iter().filter(|x| x.file.is_none() && !x.hidden).count();
        let item = self.fetch_for(r).cloned();
        let busy = self.fetch.iter().any(|f| matches!(f.state.as_str(), "queued" | "searching" | "downloading" | "verifying"));
        let pane = self.clone_for_fetch();
        let rank = r.rank;
        menu.list_section(ctx, move |mut section| {
            match item.as_ref().map(|f| f.state.as_str()) {
                None => {
                    let (one, ten, all) = (pane.clone(), pane.clone(), pane.clone());
                    let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
                    section.add_item("Fetch this song", move |ctx| {
                        one.start_fetch(ctx, vec!["add".into(), "--from".into(), p1, "--rank".into(), rank.to_string()]);
                        Ok(())
                    });
                    section.add_item(format!("Fetch the first {} missing", missing.min(10)), move |ctx| {
                        ten.start_fetch(ctx, vec!["add".into(), "--from".into(), p2, "--first".into(), "10".into()]);
                        Ok(())
                    });
                    section.add_item(format!("Fetch all {missing} missing…"), move |ctx| {
                        let message = vec![format!(
                            "Fetch all {missing} missing songs of this result?\n\nOne at a time with pauses (YouTube \
                             blocks bursts), so this takes a while. Only exact recording matches go into the library; \
                             the rest wait for review here."
                        )];
                        let go = move |ctx: &Ctx| -> Result<()> {
                            all.start_fetch(ctx, vec!["add".into(), "--from".into(), p3.clone()]);
                            Ok(())
                        };
                        modal!(
                            ctx,
                            ConfirmModal::builder()
                                .ctx(ctx)
                                .message(message)
                                .action(Action::CustomButtons {
                                    buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), ("Fetch", Box::new(go))],
                                })
                                .build()
                        );
                        Ok(())
                    });
                }
                Some("review") => {
                    let f = item.clone().expect("review item");
                    let (a, b) = (pane.clone(), pane.clone());
                    let (k1, k2) = (f.key.clone(), f.key.clone());
                    section.add_item("Accept the download into the library", move |ctx| {
                        a.fetch_decision(ctx, "accept", k1);
                        Ok(())
                    });
                    section.add_item("Reject the download (never again)", move |ctx| {
                        b.fetch_decision(ctx, "reject", k2);
                        Ok(())
                    });
                }
                Some("failed" | "rejected") => {
                    let f = item.clone().expect("failed item");
                    let a = pane.clone();
                    section.add_item("Retry fetching this song", move |ctx| {
                        a.fetch_decision(ctx, "retry", f.key.clone());
                        Ok(())
                    });
                }
                Some(_) => {}
            }
            if busy {
                let command = pane.command.clone();
                section.add_item("Stop fetching after the current song", move |_| {
                    let _ = Command::new(&command[0]).args(&command[1..]).args(["fetch", "cancel"]).status();
                    Ok(())
                });
            }
            Some(section)
        })
    }

    /// What the fetch helpers need, cheap to clone into menu callbacks.
    fn clone_for_fetch(&self) -> FetchHandle {
        FetchHandle { job: Arc::clone(&self.job), command: self.command.clone() }
    }

    fn selected(&self) -> Option<&HitsRow> {
        self.state.get_selected().and_then(|i| self.rows.get(i))
    }

    /// Jump within the visible result only; keep the filters and playback untouched.
    fn jump_to_current(&mut self, ctx: &Ctx) -> bool {
        let Some(song) = ctx.status.songid.and_then(|id| ctx.queue.iter().find(|s| s.id == id)) else {
            return false;
        };
        let matches = |r: &HitsRow| r.file.as_deref() == Some(song.file.as_str());
        let selected = self.state.get_selected();
        let idx = selected.filter(|i| self.rows.get(*i).is_some_and(matches))
            .or_else(|| self.rows.iter().position(matches));
        let Some(idx) = idx else { return false };
        let scrolloff = if selected == Some(idx) { usize::MAX } else { ctx.config.scrolloff };
        self.state.select(Some(idx), scrolloff);
        self.focus_filters = false;
        self.hover_filter = None;
        true
    }

    /// Narrow the result to the `/` search (every word in artist or title, diacritics folded, any order); keep the
    /// row of rank `keep` selected when it is still shown.
    fn refilter(&mut self, keep: Option<u32>) {
        let found = crate::ui::rormpc_filter::find(
            self.all_rows.iter().map(|r| format!("{} {}", r.artist, r.title)),
            &self.query,
        );
        self.rows = found.rows.into_iter().map(|i| self.all_rows[i].clone()).collect();
        self.state.set_content_and_viewport_len(self.rows.len(), self.state_viewport());
        let idx = keep.and_then(|k| self.rows.iter().position(|r| r.rank == k)).unwrap_or(0);
        self.state.select((!self.rows.is_empty()).then_some(idx), 0);
    }

    /// x of the ♥ column: after Rank (5), % (4) and the owned mark (1), each followed by one space.
    fn like_x(&self) -> u16 {
        self.table_area.x + 5 + 1 + 4 + 1 + 1 + 1
    }

    /// Like <-> no rating for an owned song (a disliked one becomes liked); dislike stays in the menu.
    fn toggle_like(&self, idx: usize, ctx: &Ctx) {
        let Some(r) = self.rows.get(idx) else { return };
        let Some(file) = r.file.clone() else {
            return status_info!("No local file: nothing to like (missing song)");
        };
        let liked = ctx.song_stickers(&file).and_then(|st| st.get("like")).is_some_and(|v| v == "2");
        rormpc_actions::set_like(ctx, file, if liked { "1" } else { "2" });
        status_info!("{}: {}", if liked { "Like removed" } else { "Liked ♥" }, r.title);
    }

    /// Esc: the whole result again.
    fn clear_search(&mut self, ctx: &Ctx) {
        ctx.input.clear_buffer(self.search);
        self.query.clear();
        self.typing = false;
        let keep = self.selected().map(|r| r.rank);
        self.refilter(keep);
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
                    Ok(out) if out.status.success() => {
                        job.lock().expect("hits job lock").ok_args = Some(args.clone());
                        None
                    }
                    Ok(out) => Some(
                        String::from_utf8_lossy(&out.stderr).lines().rev().find(|l| !l.trim().is_empty())
                            .unwrap_or("hits failed").to_owned(),
                    ),
                    Err(err) => Some(crate::shared::dependencies::cannot_run(&command[0], &err)),
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
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::AddGenre => {
                self.ask_genre(ctx);
                return true;
            }
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::AddArtist => {
                self.pick_artist(ctx);
                return true;
            }
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::Explore => {
                self.count_genres(ctx);
                return true;
            }
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
    /// Apply (the last row) lives in its own footer, so it neither counts nor scrolls here.
    fn scroll_filters(&mut self, delta: isize) {
        let height = usize::from(self.filter_area.height);
        let listed = self.filter_rows().len().saturating_sub(1);
        let max = listed.saturating_sub(height);
        if delta == 0 && (height == 0 || self.filter_sel >= listed) {
            // the cursor is on the Apply footer, or no list row fits: nothing to follow
        } else if delta == 0 {
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

    /// Count the library's genres (`hits genres --json`) in the background; the menu opens when they arrive.
    /// The first run looks artists up on MusicBrainz and takes minutes; later ones read the caches.
    fn count_genres(&self, ctx: &Ctx) {
        let mut ex = self.explorer.lock().expect("explorer lock");
        if ex.loading {
            return;
        }
        ex.loading = true;
        drop(ex);
        status_info!("Counting the library's genres…");
        let (explorer, command, sender) = (Arc::clone(&self.explorer), self.command.clone(), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let out = Command::new(&command[0]).args(&command[1..]).args(["genres", "--json"]).output();
            let parsed = match out {
                Ok(o) if o.status.success() => serde_json::from_slice::<GenresFile>(&o.stdout).map_err(|e| e.to_string()),
                Ok(o) => Err(last_line(&o.stderr, "hits genres failed")),
                Err(err) => Err(crate::shared::dependencies::cannot_run(&command[0], &err)),
            };
            let mut ex = explorer.lock().expect("explorer lock");
            ex.loading = false;
            match parsed {
                Ok(file) => ex.ready = Some(file),
                Err(err) => status_error!("hits genres: {err}"),
            }
            drop(ex);
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// Every genre with its song count; choosing one asks what to do with it.
    fn open_explorer(&self, ctx: &Ctx, file: GenresFile) {
        let explorer = Arc::clone(&self.explorer);
        let title = format!("{} songs, {} without a genre · songs (own tags) plays", file.total, file.unknown);
        let menu = MenuModal::new(ctx)
            .width(60)
            .list_section(ctx, move |mut section| {
                section.add_item(title, |_| Ok(()));
                for g in file.genres {
                    let explorer = Arc::clone(&explorer);
                    let label = format!(
                        "{:>4} ({:>3}) {:>5}  {}{}",
                        g.songs,
                        g.recording,
                        g.plays,
                        g.name,
                        if g.pinned { "  [pinned]" } else { "" }
                    );
                    section.add_item(label, move |ctx| {
                        open_genre_actions(ctx, explorer, g.name.clone(), g.pinned);
                        Ok(())
                    });
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    /// Act on a genre picked in the explorer: filter by it alone (and run hits), or pin / unpin its checkbox.
    fn explorer_pick(&mut self, ctx: &Ctx, genre: String, pick: ExplorerPick) {
        let Some(filters) = self.filters.as_mut() else { return };
        match pick {
            ExplorerPick::Filter | ExplorerPick::FilterLikes => {
                filters.genres.iter_mut().for_each(|g| g.1 = 0);
                filters.add_genres(&format!("+{genre}"));
                if pick == ExplorerPick::FilterLikes {
                    filters.source = Source::Likes;
                    filters.by_range = false;
                    filters.decades = [false; 8]; // all years
                    filters.tops = [false; 3]; // every liked song with it, not a percentile of a few
                }
                self.apply(ctx);
            }
            ExplorerPick::Pin | ExplorerPick::Unpin => {
                if pick == ExplorerPick::Pin && !filters.genres.iter().any(|(g, _)| *g == genre) {
                    filters.genres.push((genre.clone(), 0));
                }
                let verb = if pick == ExplorerPick::Pin { "pin" } else { "unpin" };
                let command = self.command.clone();
                std::thread::spawn(move || {
                    match Command::new(&command[0]).args(&command[1..]).args(["genres", verb, &genre]).output() {
                        Ok(o) if o.status.success() => status_info!("{verb}ned {genre}"),
                        _ => status_error!("hits genres {verb} {genre} failed"),
                    }
                });
            }
        }
    }

    /// Ask for genres that have no checkbox; they are added as rows ("+name" includes, "-name" excludes).
    fn ask_genre(&self, ctx: &Ctx) {
        let input = Arc::clone(&self.genre_input);
        let sender = ctx.app_event_sender.clone();
        modal!(
            ctx,
            InputModal::new(ctx)
                .title("Other genres, e.g. italo-disco, -schlager")
                .input_label("Genres:")
                .confirm_label("Add")
                .on_confirm(move |_, value| {
                    *input.lock().expect("genre input lock") = Some(value.to_owned());
                    let _ = sender.send(AppEvent::RequestRender);
                    Ok(())
                })
        );
    }

    /// "+ artist…": the artists of the result's whole cohort (before the Top % cut) with their song counts, most
    /// songs first; `/` in the menu searches it. The pick is added as "+artist" (Space then cycles it).
    fn pick_artist(&self, ctx: &Ctx) {
        if self.cohort_artists.is_empty() {
            return status_info!("No artists in this result yet (Apply first)");
        }
        let input = Arc::clone(&self.artist_input);
        let sender = ctx.app_event_sender.clone();
        let artists = self.cohort_artists.clone();
        let title = format!("{} artists in this cohort · / searches", artists.len());
        let menu = MenuModal::new(ctx)
            .width(60)
            .list_section(ctx, move |mut section| {
                section.add_item(title, |_| Ok(()));
                for a in artists {
                    let (input, sender) = (Arc::clone(&input), sender.clone());
                    section.add_item(format!("{:>3}  {}", a.songs, a.name), move |_| {
                        *input.lock().expect("artist input lock") = Some(a.name.clone());
                        let _ = sender.send(AppEvent::RequestRender);
                        Ok(())
                    });
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    fn render_filters(&self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let Some(filters) = &self.filters else { return };
        let rows = filters.rows();
        let apply_idx = rows.len().saturating_sub(1);
        let lines: Vec<Line> = rows
            .iter()
            .enumerate()
            .take(apply_idx)
            .skip(self.filter_offset)
            .map(|(i, row)| {
                let text = filters.line(*row);
                let label_style = if matches!(
                    row,
                    FilterRow::Heading(_) | FilterRow::Source | FilterRow::Sort | FilterRow::Mode | FilterRow::Apply
                ) {
                    ctx.config.theme.preview_label_style
                } else if !filters.enabled(*row) {
                    Style::default().add_modifier(Modifier::DIM) // "× clear …" with nothing to clear
                } else {
                    Style::default()
                };
                let hover = self.hover_filter == Some(i) && filters.hoverable(*row);
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
                if hover {
                    // underline the words only: never the indent or the box, whose look is its state
                    // (BOLD too, since some terminals don't draw underlines)
                    let hovered = Style::default().add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
                    let (boxed, name) = if control.starts_with('[') { control.split_at(3) } else { ("", control) };
                    let (words, indent) = if boxed.is_empty() { (label, "") } else { (name, boxed) };
                    let lead = words.len() - words.trim_start().len();
                    let style = if boxed.is_empty() { label_style } else { Style::default() };
                    let mut spans = vec![gutter];
                    if !boxed.is_empty() {
                        spans.push(Span::styled(label.to_owned(), label_style));
                        spans.push(Span::styled(indent.to_owned(), control_style));
                    }
                    spans.push(Span::styled(words[..lead].to_owned(), style));
                    spans.push(Span::styled(words[lead..].to_owned(), style.patch(hovered)));
                    return Line::from(spans);
                }
                Line::from(vec![
                    gutter,
                    Span::styled(label.to_owned(), label_style),
                    Span::styled(control.to_owned(), control_style),
                ])
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
        frame.render_widget(Paragraph::new(self.apply_line(filters, apply_idx, ctx)), self.apply_area);
    }

    /// The sticky Apply footer: the button plus whether pressing it would change anything.
    fn apply_line(&self, filters: &Filters, apply_idx: usize, ctx: &Ctx) -> Line<'static> {
        let cursor = self.filter_sel == apply_idx;
        let gutter = match (cursor, self.focus_filters) {
            (true, true) => Span::styled("›", ctx.config.theme.preview_label_style.add_modifier(Modifier::BOLD)),
            (true, false) => Span::styled("›", Style::default().add_modifier(Modifier::DIM)),
            _ => Span::raw(" "),
        };
        let mut button = ctx.config.theme.preview_label_style.add_modifier(Modifier::REVERSED);
        if cursor && self.focus_filters {
            button = button.add_modifier(Modifier::BOLD);
        }
        let (running, queued) = {
            let j = self.job.lock().expect("hits job lock");
            (j.running, j.queued.is_some())
        };
        let changed = self.applied_args.as_ref() != Some(&filters.args(&self.path.to_string_lossy()));
        let state = match (running, queued, changed) {
            (true, true, _) => " running, then again",
            (true, false, _) => " running…",
            (false, _, true) => " • changed",
            (false, _, false) => "",
        };
        Line::from(vec![
            gutter,
            Span::raw(" "),
            Span::styled("[ Apply ]", button),
            Span::styled(state, Style::default().add_modifier(Modifier::DIM)),
        ])
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
            let what = format!("'{}'", r.title);
            let genre_what = what.clone();
            menu = menu.list_section(ctx, move |mut section| {
                for (label, value) in [("Like ♥", "2"), ("Dislike ✗", "0"), ("Clear like", "1")] {
                    let file = like_file.clone();
                    section.add_item(format!("{label}{hint}"), move |ctx| {
                        rormpc_actions::set_like(ctx, file, value);
                        Ok(())
                    });
                }
                let next_file = like_file.clone();
                section.add_item("Play next", move |ctx| {
                    crate::ui::rormpc_upnext::play_next(ctx, vec![next_file]);
                    Ok(())
                });
                let playlist_file = like_file.clone();
                section.add_item("Add to playlist…", move |ctx| {
                    crate::ui::rormpc_playlists::open_add_to_playlist(ctx, vec![playlist_file], what);
                    Ok(())
                });
                let genre_file = like_file.clone();
                section.add_item("Tags…", move |ctx| {
                    crate::ui::rormpc_tags::open_tags_menu(ctx, like_file);
                    Ok(())
                });
                section.add_item("Pin genre in Hits…", move |ctx| {
                    crate::ui::rormpc_genres::open_pin_menu_for_file(ctx, genre_file, genre_what);
                    Ok(())
                });
                Some(section)
            });
        }
        if r.file.is_none() && !r.hidden {
            menu = self.fetch_section(ctx, menu, &r);
            // not in the library: the genres `hits` gave the chart row
            let (genres, what) = (r.genres.clone(), format!("'{}'", r.title));
            menu = menu.list_section(ctx, move |section| {
                Some(section.item("Pin genre in Hits…", move |ctx| {
                    crate::ui::rormpc_genres::open_pin_menu(ctx, what, genres, "chart");
                    Ok(())
                }))
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
        // the whole result as a playlist: owned rows in ranking order, missing ones skipped (and said so)
        let owned: Vec<String> = self.all_rows.iter().filter_map(|x| x.file.clone()).collect();
        let missing = self.all_rows.len() - owned.len();
        if !owned.is_empty() {
            let (name, files) = (self.label.clone(), owned.clone());
            let label = format!("Play these {} songs (as the source)…", owned.len());
            menu = menu.list_section(ctx, move |mut section| {
                section.add_item(label, move |ctx| {
                    crate::ui::rormpc_upnext::play_hits_source(ctx, name.clone(), files.clone());
                    Ok(())
                });
                Some(section)
            });
        }
        if !owned.is_empty() {
            let what = format!("{} owned songs of this result ({missing} missing skipped)", owned.len());
            let label = format!("Add all {} owned rows to playlist…", owned.len());
            menu = menu.list_section(ctx, move |mut section| {
                section.add_item(label, move |ctx| {
                    crate::ui::rormpc_playlists::open_add_to_playlist(ctx, owned, what);
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
            if let Some(reason) = &r.reason {
                field("Why", reason.clone())
            } else {
                field("Year-end charts", years)
            },
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
            None => {
                lines.push(field("In library", "no (not found in MPD)".to_owned()));
                if let Some(f) = self.fetch_for(r) {
                    lines.push(field("Fetch", f.state.clone()));
                    for detail in [&f.reason, &f.error].into_iter().flatten() {
                        lines.push(Line::from(Span::styled(detail.clone(), dim)));
                    }
                    if let Some(found) = &f.found {
                        lines.push(field("Identified as", found.clone()));
                    }
                    if let Some(candidate) = &f.candidate {
                        lines.push(field("From", candidate.clone()));
                    }
                    if f.state == "review" {
                        lines.push(Line::from(Span::styled("menu: Accept / Reject the fetched file", dim)));
                    }
                } else {
                    lines.push(Line::default());
                    lines.push(Line::from(Span::styled("menu: Fetch this song", dim)));
                }
            }
        }
        if r.hidden {
            lines.push(Line::from(Span::styled("hidden from Hits (menu: Unhide)", dim)));
        }
        lines
    }
}

/// What to do with a genre chosen in the explorer.
fn open_genre_actions(ctx: &Ctx, explorer: Arc<Mutex<Explorer>>, genre: String, pinned: bool) {
    let sender = ctx.app_event_sender.clone();
    let choose = move |pick: ExplorerPick| {
        let (explorer, genre, sender) = (Arc::clone(&explorer), genre.clone(), sender.clone());
        move |_: &Ctx| -> Result<()> {
            explorer.lock().expect("explorer lock").pick = Some((genre.clone(), pick));
            let _ = sender.send(AppEvent::RequestRender);
            Ok(())
        }
    };
    let (filter, likes, pin) = (choose(ExplorerPick::Filter), choose(ExplorerPick::FilterLikes),
        choose(if pinned { ExplorerPick::Unpin } else { ExplorerPick::Pin }));
    let menu = MenuModal::new(ctx)
        .list_section(ctx, move |mut section| {
            section.add_item("Top hits with this genre (chart)", filter);
            section.add_item("My liked songs with this genre", likes);
            section.add_item(if pinned { "Unpin its checkbox" } else { "Pin as a checkbox" }, pin);
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

impl HitsPane {
    /// " · fetch: 2 queued, 1 review" for the songs of this result that are in the fetch queue.
    fn fetch_summary(&self) -> String {
        let mut counts: Vec<(&str, usize)> = Vec::new();
        for r in self.rows.iter().filter(|r| r.file.is_none()) {
            if let Some(f) = self.fetch_for(r) {
                let state = match f.state.as_str() {
                    "searching" | "downloading" | "verifying" => "fetching",
                    s => s,
                };
                match counts.iter_mut().find(|(s, _)| *s == state) {
                    Some((_, n)) => *n += 1,
                    None => counts.push((state, 1)),
                }
            }
        }
        if counts.is_empty() {
            return String::new();
        }
        let parts: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {s}")).collect();
        format!(" · fetch: {}", parts.join(", "))
    }
}

/// The fetch actions without the pane, for menu callbacks (they outlive the borrow of the pane).
#[derive(Debug, Clone)]
struct FetchHandle {
    job: Arc<Mutex<Job>>,
    command: Vec<String>,
}

impl FetchHandle {
    /// `hits fetch add` with these arguments, then the worker (`hits fetch run`; a second one exits at once),
    /// in a background thread. When songs arrived, `hits` runs again so they show as owned.
    fn start_fetch(&self, ctx: &Ctx, add: Vec<String>) {
        let (job, command, sender) = (Arc::clone(&self.job), self.command.clone(), ctx.app_event_sender.clone());
        let mut args = vec!["fetch".to_owned()];
        args.extend(add);
        std::thread::spawn(move || {
            let hits = |args: &[String]| Command::new(&command[0]).args(&command[1..]).args(args).output();
            if args.len() > 1 {
                match hits(&args) {
                    Ok(out) if out.status.success() => status_info!("{}", last_line(&out.stdout, "queued")),
                    Ok(out) => return status_error!("hits fetch: {}", last_line(&out.stderr, "failed")),
                    Err(err) => return status_error!("{}", crate::shared::dependencies::cannot_run(&command[0], &err)),
                }
            }
            match hits(&["fetch".to_owned(), "run".to_owned()]) {
                Ok(out) if out.status.success() => {
                    let summary = last_line(&out.stdout, "fetch: nothing to do");
                    status_info!("{summary}");
                    if !summary.contains("fetch: 0 new") && summary.starts_with("fetch:") {
                        job.lock().expect("hits job lock").rerun = true;
                    }
                }
                Ok(out) => status_error!("hits fetch: {}", last_line(&out.stderr, "failed")),
                Err(err) => status_error!("{}", crate::shared::dependencies::cannot_run(&command[0], &err)),
            }
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// accept / reject / retry one queued song, then (retry) start the worker or (accept) rerun `hits`.
    fn fetch_decision(&self, ctx: &Ctx, verb: &'static str, key: String) {
        let (job, command, sender) = (Arc::clone(&self.job), self.command.clone(), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let ok = Command::new(&command[0]).args(&command[1..]).args(["fetch", verb, &key]).status().is_ok_and(|s| s.success());
            if !ok {
                status_error!("hits fetch {verb} failed");
            } else if verb == "accept" {
                job.lock().expect("hits job lock").rerun = true;
            }
            let _ = sender.send(AppEvent::RequestRender);
        });
        if verb == "retry" {
            self.start_fetch(ctx, Vec::new());
        }
    }
}

fn last_line(bytes: &[u8], fallback: &str) -> String {
    String::from_utf8_lossy(bytes).lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(fallback).to_owned()
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
        if let Some(args) = self.job.lock().expect("hits job lock").ok_args.take() {
            self.applied_args = Some(args);
        }
        let picked = self.artist_input.lock().expect("artist input lock").take();
        if let (Some(name), Some(filters)) = (picked, self.filters.as_mut()) {
            let i = filters.add_artist(&name, 1);
            let rows = filters.rows();
            self.filter_sel = rows.iter().position(|r| *r == FilterRow::Artist(i)).unwrap_or(self.filter_sel);
        }
        let typed = self.genre_input.lock().expect("genre input lock").take();
        if let (Some(spec), Some(filters)) = (typed, self.filters.as_mut()) {
            if let Some(first) = filters.add_genres(&spec) {
                let rows = filters.rows();
                self.filter_sel = rows.iter().position(|r| *r == FilterRow::Genre(first)).unwrap_or(self.filter_sel);
            }
        }
        let (ready, pick) = {
            let mut ex = self.explorer.lock().expect("explorer lock");
            (ex.ready.take(), ex.pick.take())
        };
        if let Some(file) = ready {
            self.open_explorer(ctx, file);
        }
        if let Some((genre, pick)) = pick {
            self.explorer_pick(ctx, genre, pick);
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
        // Apply gets its row first, so even a tiny pane shows it
        let [list_area, apply_area] = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(filter_area);
        if list_area != self.filter_area {
            self.hover_filter = None; // resized: the row under the pointer is another one now
        }
        self.filter_area = list_area;
        self.apply_area = apply_area;
        self.scroll_filters(0); // the pane may have been resized
        self.render_filters(frame, list_area, ctx);
        let searching = self.typing || !self.query.is_empty();
        let [search_area, table_area, footer] = Layout::vertical([
            Constraint::Length(u16::from(searching)),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .areas(main);
        if searching {
            let text = format!(" Search: {}{}", self.query, if self.typing { "▏" } else { "" });
            let count = format!("{} shown / {} results ", self.rows.len(), self.all_rows.len());
            let [t, c] = Layout::horizontal([Constraint::Min(1), Constraint::Length(count.chars().count() as u16)])
                .areas(search_area);
            let style = if self.typing { ctx.config.theme.highlight_border_style } else { ctx.config.as_text_style() };
            frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), t);
            frame.render_widget(Paragraph::new(Line::from(count)), c);
        }
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.rows.len(), self.state_viewport());

        self.reload_fetch();
        self.reload_pins();
        let dim = Style::default().add_modifier(Modifier::DIM);
        // the source being played, when it is a Hits snapshot: rows outside it and rows heard in this round
        let source = crate::ui::rormpc_upnext::source_info().filter(|(kind, _, _)| kind == "hits");
        let snapshot: std::collections::HashSet<&str> =
            source.as_ref().map(|(_, _, f)| f.iter().map(String::as_str).collect()).unwrap_or_default();
        let shuffle = crate::ui::rormpc_player::shuffle_state();
        let heard: std::collections::HashSet<&str> =
            shuffle.round.as_ref().map(|r| r.heard.iter().map(String::as_str).collect()).unwrap_or_default();
        let hover = self.hover_like;
        let rows = self.rows.iter().enumerate().map(|(i, r)| {
            let owned = r.file.is_some();
            let missing_mark = self.fetch_for(r).map_or("✗", |f| fetch_mark(&f.state));
            let like = r.file.as_deref().and_then(|f| ctx.song_stickers(f)).and_then(|st| st.get("like").cloned());
            let like_cell = match (owned, like.as_deref()) {
                (false, _) => Cell::from(Span::styled("·", dim)),
                (true, Some("2")) => Cell::from("♥"),
                (true, Some("0")) => Cell::from("✗"),
                // the liked glyph, dimmed: same shape, so it reads "click to like" (♡ is narrower in some fonts)
                (true, _) if hover == Some(i) => Cell::from(Span::styled("♥", Style::default().add_modifier(Modifier::DIM))),
                (true, _) => Cell::from(""),
            };
            let state = match r.file.as_deref() {
                None => String::new(),
                Some(f) => match crate::ui::rormpc_player::cooldown_days(f) {
                    Some(d) => format!("⏳{:.0}d", d.ceil()),
                    None if source.is_some() && !snapshot.contains(f) => "·".to_owned(),
                    None if source.is_some() && heard.contains(f) => "heard".to_owned(),
                    None => String::new(),
                },
            };
            let next = r.file.as_deref().and_then(|f| crate::ui::rormpc_player::next_marker_for_file(ctx, f));
            Row::new(vec![
                Cell::from(format!("#{}", r.rank)),
                Cell::from(format!("{:.0}%", r.pct.ceil())),
                Cell::from(if r.hidden { "h" } else if owned { "✓" } else { missing_mark }),
                like_cell,
                Cell::from(next.unwrap_or_default()),
                Cell::from(r.artist.clone()),
                Cell::from(r.title.clone()),
                Cell::from(if r.year > 0 { r.year.to_string() } else { String::new() }),
                Cell::from(if owned && r.plays > 0 { r.plays.to_string() } else { String::new() }),
                Cell::from(Span::styled(state, dim)),
            ])
            .style(if owned && !r.hidden { Style::default() } else { dim })
        });
        let header = Row::new(["Rank", "%", "", "♥", "Next", "Artist", "Title", "Year", "Plays", "State"])
            .style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(4),
            Constraint::Percentage(35),
            Constraint::Percentage(65),
            Constraint::Length(4),
            Constraint::Length(5),
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
                    " {} · {} hits · {} in library · updated {}{}",
                    self.label,
                    self.rows.len(),
                    owned,
                    self.generated_at.replace('T', " "),
                    self.fetch_summary()
                ),
                dim,
            ),
        };
        // what plays vs what is browsed: the snapshot is never changed by moving a filter
        let playing = match &source {
            Some((_, name, files)) => {
                let round = shuffle.round.as_ref().map_or(String::new(), |r| {
                    if r.done { " · round done (Up next menu: new round)".to_owned() } else { format!(" · heard {}/{}", r.heard.len(), r.total) }
                });
                let browsing = if *name == self.label { String::new() } else { " · browsing other results (Ctrl-z: Play these results)".to_owned() };
                format!(" Playing: Hits · {name} · {} playable{round}{browsing}", files.len())
            }
            None => " Ctrl-z: Play these results (as the source) · / search · click ♥ or r to like".to_owned(),
        };
        frame.render_widget(Paragraph::new(vec![Line::from(status), Line::from(Span::styled(playing, dim))]), footer);
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
        if self.hover_filter.is_some()
            && (!self.filter_area.contains(event.into()) || !matches!(event.kind, MouseEventKind::Moved))
        {
            self.hover_filter = None; // left the column, clicked or scrolled: the next move sets it again
            ctx.render()?;
        }
        if self.apply_area.contains(event.into()) {
            // a click applies; the wheel does nothing over the footer
            if matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::DoubleClick) {
                self.focus_filters = true;
                self.filter_sel = self.filter_rows().len().saturating_sub(1);
                self.apply(ctx);
                ctx.render()?;
            }
            return Ok(());
        }
        if self.filter_area.contains(event.into()) {
            match event.kind {
                MouseEventKind::Moved => {
                    let idx = self.filter_offset + usize::from(event.y.saturating_sub(self.filter_area.y));
                    let rows = self.filter_rows();
                    let hover = rows.get(idx).filter(|r| self.filters.as_ref().is_some_and(|f| f.hoverable(**r))).map(|_| idx);
                    if hover == self.hover_filter {
                        return Ok(());
                    }
                    self.hover_filter = hover;
                }
                MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                    let idx = self.filter_offset + usize::from(event.y.saturating_sub(self.filter_area.y));
                    let clickable = |row: &FilterRow| !matches!(row, FilterRow::Heading(_) | FilterRow::Apply);
                    if self.filter_rows().get(idx).is_some_and(clickable) {
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
        let row = usize::from(event.y.saturating_sub(self.table_area.y + 1)); // +1: header row
        let on_like = event.x == self.like_x() && event.y > self.table_area.y;
        if matches!(event.kind, MouseEventKind::Moved) {
            let hover = if on_like { self.state.get_at_rendered_row(row) } else { None };
            if hover != self.hover_like {
                self.hover_like = hover;
                ctx.render()?;
            }
            return Ok(());
        }
        self.focus_filters = false;
        match event.kind {
            // a click on the ♥ cell only toggles the like: it neither selects nor plays
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick if on_like => {
                if let Some(idx) = self.state.get_at_rendered_row(row) {
                    self.toggle_like(idx, ctx);
                }
            }
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

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &mut Ctx) -> Result<()> {
        match kind {
            InputResultEvent::Push | InputResultEvent::Pop => {
                self.query = ctx.input.value(self.search);
                let keep = self.selected().map(|r| r.rank);
                self.refilter(keep);
            }
            InputResultEvent::Confirm => self.typing = false, // Enter keeps the search
            InputResultEvent::Cancel => self.clear_search(ctx),
            InputResultEvent::NoChange => {}
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_insert_nav(&mut self, down: bool, handled: &mut bool, ctx: &mut Ctx) -> Result<()> {
        if self.typing {
            let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
            if down { self.state.next(scrolloff, wrap) } else { self.state.prev(scrolloff, wrap) }
            *handled = true;
            ctx.render()?;
        }
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        if let Some(action) = event.claim_queue() {
            if matches!(action, QueueActions::JumpToCurrent) {
                if self.jump_to_current(ctx) {
                    ctx.render()?;
                }
                return Ok(()); // recognized even without a match: never fall through to playback
            }
            event.abandon(); // Queue-only actions must not consume common or global bindings
        }
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        self.hover_filter = None; // a key press: the cursor is what counts, until the mouse moves again
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
            CommonAction::EnterSearch => {
                self.typing = true;
                ctx.input.insert_mode(self.search);
            }
            CommonAction::Close if !self.query.is_empty() => self.clear_search(ctx),
            CommonAction::Rate { .. } => {
                if let Some(i) = self.state.get_selected() {
                    self.toggle_like(i, ctx);
                }
            }
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
const GENRES: [&str; 19] = [
    "rock", "pop", "hip hop", "r&b", "soul", "dance", "electronic", "disco", "funk", "country", "metal",
    "folk", "latin", "jazz", "blues", "punk", "reggae", "classical", "soundtrack",
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
    /// sets every genre row back to off, including exclusions scrolled out of view
    ClearGenres,
    /// opens an input for genres without a checkbox
    AddGenre,
    /// the genre explorer: every genre of the library with counts
    Explore,
    /// three-state like a genre: +include / -exclude / off
    Artist(usize),
    ClearArtists,
    /// opens the artist picker
    AddArtist,
    Owned,
    ShowHidden,
    Apply,
}

/// Where the ranked songs come from (`hits --source`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Billboard,
    Likes,
    Recs,
    /// every library song, by my plays
    Library,
    /// my own charts: songs by my plays in the chosen listening years
    Mine,
}

impl Source {
    const ALL: [Source; 5] = [Source::Billboard, Source::Mine, Source::Library, Source::Likes, Source::Recs];

    fn label(self) -> &'static str {
        match self {
            Source::Billboard => "Billboard US",
            Source::Likes => "my likes",
            Source::Recs => "recommended",
            Source::Library => "whole library",
            Source::Mine => "my charts",
        }
    }

    /// The next (delta > 0) or previous source, wrapping around.
    fn next(self, delta: i32) -> Source {
        let i = Self::ALL.iter().position(|s| *s == self).unwrap_or(0) as i32;
        Self::ALL[(i + delta).rem_euclid(Self::ALL.len() as i32) as usize]
    }
}

/// What the filter column edits; turned into `hits` arguments on Apply.
#[derive(Debug, Clone)]
struct Filters {
    source: Source,
    /// likes only: false = by plays, true = rediscover
    rediscover: bool,
    by_range: bool,
    decades: [bool; 8],
    from: i32,
    to: i32,
    tops: [bool; 3],
    /// the pinned genres (or GENRES) first, then typed ones; -1 exclude, 0 off, 1 include
    genres: Vec<(String, i8)>,
    /// artists picked in "+ artist…": -1 exclude, 0 off, 1 include
    artists: Vec<(String, i8)>,
    owned: bool,
    show_hidden: bool,
}

impl Default for Filters {
    fn default() -> Self {
        let mut decades = [false; 8];
        decades[3] = true; // 1980s
        Self { source: Source::Billboard, rediscover: false, by_range: false, decades, from: 1985, to: 1992, tops: [true, false, false], genres: pinned_genres().into_iter().map(|g| (g, 0)).collect(), artists: Vec::new(), owned: false, show_hidden: false }
    }
}

impl Filters {
    fn rows(&self) -> Vec<FilterRow> {
        let mut rows = vec![FilterRow::Source];
        if matches!(self.source, Source::Likes | Source::Library) {
            rows.push(FilterRow::Sort);
        }
        // recommendations have no year to filter on
        if self.source != Source::Recs {
            rows.push(FilterRow::Mode);
            if self.by_range {
                rows.extend([FilterRow::From, FilterRow::To]);
            } else {
                rows.extend((0..DECADES.len()).map(FilterRow::Decade));
            }
        }
        rows.push(FilterRow::Heading("Top %"));
        rows.extend((0..TOPS.len()).map(FilterRow::Top));
        // the heading counts the ticked rows (see `line`): the list is taller than the screen, and a "[-] country"
        // below the fold kept filtering while the visible rows were all off
        rows.extend([FilterRow::Heading("Genres"), FilterRow::ClearGenres]);
        rows.extend((0..self.genres.len()).map(FilterRow::Genre));
        rows.extend([FilterRow::AddGenre, FilterRow::Explore]);
        rows.push(FilterRow::Heading("Artists"));
        if !self.artists.is_empty() {
            rows.push(FilterRow::ClearArtists);
        }
        rows.extend((0..self.artists.len()).map(FilterRow::Artist));
        rows.push(FilterRow::AddArtist);
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

    /// "Genres: all", "Genres: +2", "Genres: -1" or "Genres: +2 -1": counts, since names don't fit 24 columns.
    fn genres_heading(&self) -> String {
        let count = |sign: i8| self.genres.iter().filter(|(_, s)| *s == sign).count();
        let parts: Vec<String> = [(count(1), '+'), (count(-1), '-')]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, sign)| format!("{sign}{n}"))
            .collect();
        format!("Genres: {}", if parts.is_empty() { "all".to_owned() } else { parts.join(" ") })
    }

    /// "Artists: all", "Artists: +2 -1".
    fn artists_heading(&self) -> String {
        let count = |sign: i8| self.artists.iter().filter(|(_, s)| *s == sign).count();
        let parts: Vec<String> = [(count(1), '+'), (count(-1), '-')]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, sign)| format!("{sign}{n}"))
            .collect();
        format!("Artists: {}", if parts.is_empty() { "all".to_owned() } else { parts.join(" ") })
    }

    /// Semicolon-separated: a name may hold a comma ("Earth, Wind & Fire").
    fn artist_spec(&self) -> String {
        self.artists
            .iter()
            .filter(|(_, s)| *s != 0)
            .map(|(a, s)| format!("{}{a}", if *s > 0 { '+' } else { '-' }))
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Set an artist's state, adding its row when it has none; returns its index.
    fn add_artist(&mut self, name: &str, sign: i8) -> usize {
        let i = match self.artists.iter().position(|(a, _)| a.eq_ignore_ascii_case(name)) {
            Some(i) => i,
            None => {
                self.artists.push((name.to_owned(), 0));
                self.artists.len() - 1
            }
        };
        self.artists[i].1 = sign;
        i
    }

    /// Comma-separated, so names with spaces ("hip hop") survive the round trip through `hits`.
    fn genre_spec(&self) -> String {
        self.genres
            .iter()
            .filter(|(_, s)| *s != 0)
            .map(|(g, s)| format!("{}{g}", if *s > 0 { '+' } else { '-' }))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Set the genres of a spec like "+italo-disco, -schlager" (no sign = include), adding rows for names
    /// without one. Returns the index of the first genre touched.
    fn add_genres(&mut self, spec: &str) -> Option<usize> {
        let mut first = None;
        for (sign, name) in parse_genre_spec(spec) {
            let i = match self.genres.iter().position(|(g, _)| *g == name) {
                Some(i) => i,
                None => {
                    self.genres.push((name, 0));
                    self.genres.len() - 1
                }
            };
            self.genres[i].1 = sign;
            first.get_or_insert(i);
        }
        first
    }

    fn args(&self, json: &str) -> Vec<String> {
        let tops: Vec<String> =
            TOPS.iter().zip(self.tops).filter(|(_, on)| *on).map(|((lo, hi), _)| format!("{lo}-{hi}")).collect();
        let mut args = Vec::new();
        match self.source {
            Source::Billboard => {}
            Source::Likes => {
                args.extend(["--source".to_owned(), "likes".to_owned(), "--sort".to_owned()]);
                args.push(if self.rediscover { "rediscover" } else { "plays" }.to_owned());
            }
            Source::Recs => args.extend(["--source".to_owned(), "recs".to_owned()]),
            Source::Library => {
                args.extend(["--source".to_owned(), "library".to_owned(), "--sort".to_owned()]);
                args.push(if self.rediscover { "rediscover" } else { "plays" }.to_owned());
            }
            Source::Mine => args.extend(["--source".to_owned(), "mine".to_owned()]),
        }
        // likes, library and my charts without any decade ticked = all years (a chart needs a period);
        // recommendations have no year
        let all_years = matches!(self.source, Source::Likes | Source::Library | Source::Mine)
            && !self.by_range
            && !self.decades.iter().any(|d| *d);
        if self.source != Source::Recs && !all_years {
            args.extend(["--years".to_owned(), self.years()]);
        }
        args.push("--top".to_owned());
        args.push(if tops.is_empty() { "1-100".to_owned() } else { tops.join(",") });
        let genres = self.genre_spec();
        if !genres.is_empty() {
            // one token: argparse takes a separate value starting with '-' ("-country") for an option
            args.push(format!("--genre={genres}"));
        }
        let artists = self.artist_spec();
        if !artists.is_empty() {
            args.push(format!("--artist={artists}"));
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
        f.add_genres(args.genre.as_deref().unwrap_or_default());
        for tok in args.artist.as_deref().unwrap_or_default().split(';') {
            let tok = tok.trim();
            match tok.chars().next() {
                Some('-') => _ = f.add_artist(tok[1..].trim(), -1),
                Some('+') => _ = f.add_artist(tok[1..].trim(), 1),
                Some(_) => _ = f.add_artist(tok, 1),
                None => {}
            }
        }
        f.owned = args.owned;
        f.show_hidden = args.show_hidden;
        f.source = match args.source.as_deref() {
            Some("likes") => Source::Likes,
            Some("recs") => Source::Recs,
            Some("library") => Source::Library,
            Some("mine") => Source::Mine,
            _ => Source::Billboard,
        };
        f.rediscover = args.sort.as_deref() == Some("rediscover");
        if matches!(f.source, Source::Likes | Source::Library | Source::Mine) && args.period.is_none() {
            f.decades = [false; 8]; // all years
        }
        f
    }

    fn line(&self, row: FilterRow) -> String {
        let check = |on: bool| if on { "[x]" } else { "[ ]" };
        match row {
            FilterRow::Source => format!("Source: ‹{}›", self.source.label()),
            FilterRow::Sort => format!("Sort:   {}", if self.rediscover { "‹rediscover›" } else { "‹by plays›" }),
            // my charts filter the years I listened, the other sources the songs' release years
            FilterRow::Mode => format!(
                "{} {}",
                if self.source == Source::Mine { "Listened:" } else { "Period:" },
                if self.by_range { "‹year range›" } else { "‹decades›" }
            ),
            FilterRow::Decade(i) => format!("  {} {}s", check(self.decades[i]), DECADES[i]),
            FilterRow::From => format!("  from ‹ {} ›", self.from),
            FilterRow::To => format!("  to   ‹ {} ›", self.to),
            // every box sits at the same 2-cell indent under its heading: a hanging label per group made the
            // columns step like an expandable tree
            FilterRow::Heading("Genres") => self.genres_heading(),
            FilterRow::Heading("Artists") => self.artists_heading(),
            FilterRow::Artist(i) => {
                let mark = match self.artists[i].1 { 1 => "+", -1 => "-", _ => " " };
                format!("  [{mark}] {}", self.artists[i].0)
            }
            FilterRow::ClearArtists => "  × clear artists".to_owned(),
            FilterRow::AddArtist => "  + artist…".to_owned(),
            FilterRow::Heading(title) => title.to_owned(),
            FilterRow::Top(i) => format!("  {} {}-{}%", check(self.tops[i]), TOPS[i].0, TOPS[i].1),
            FilterRow::Genre(i) => {
                let mark = match self.genres[i].1 { 1 => "+", -1 => "-", _ => " " };
                format!("  [{mark}] {}", self.genres[i].0)
            }
            FilterRow::ClearGenres => "  × clear genres".to_owned(),
            FilterRow::AddGenre => "  + other genre…".to_owned(),
            FilterRow::Explore => "  ⋯ explore genres…".to_owned(),
            FilterRow::Owned => format!("  {} owned only", check(self.owned)),
            FilterRow::ShowHidden => format!("  {} show hidden", check(self.show_hidden)),
            FilterRow::Apply => "  [ Apply ]".to_owned(),
        }
    }

    /// False for "× clear genres" / "× clear artists" when no box is + or -: drawn dim, Enter does nothing.
    fn enabled(&self, row: FilterRow) -> bool {
        match row {
            FilterRow::ClearGenres => self.genres.iter().any(|(_, s)| *s != 0),
            FilterRow::ClearArtists => self.artists.iter().any(|(_, s)| *s != 0),
            _ => true,
        }
    }

    /// Rows that underline under the mouse: actions and the label of a checkbox, not headings, the ‹…› cyclers,
    /// Apply (its own button look) or a disabled row.
    fn hoverable(&self, row: FilterRow) -> bool {
        self.enabled(row)
            && matches!(
                row,
                FilterRow::ClearGenres
                    | FilterRow::ClearArtists
                    | FilterRow::AddGenre
                    | FilterRow::Explore
                    | FilterRow::AddArtist
                    | FilterRow::Decade(_)
                    | FilterRow::Top(_)
                    | FilterRow::Genre(_)
                    | FilterRow::Artist(_)
                    | FilterRow::Owned
                    | FilterRow::ShowHidden
            )
    }

    /// Space / Enter on a row.
    fn toggle(&mut self, row: FilterRow) {
        match row {
            FilterRow::Source => self.source = self.source.next(1),
            FilterRow::Sort => self.rediscover = !self.rediscover,
            FilterRow::Mode => self.by_range = !self.by_range,
            FilterRow::Decade(i) => self.decades[i] = !self.decades[i],
            FilterRow::Top(i) => self.tops[i] = !self.tops[i],
            FilterRow::Genre(i) => self.genres[i].1 = match self.genres[i].1 { 0 => 1, 1 => -1, _ => 0 },
            FilterRow::ClearGenres => self.genres.iter_mut().for_each(|(_, s)| *s = 0),
            FilterRow::Artist(i) => self.artists[i].1 = match self.artists[i].1 { 0 => 1, 1 => -1, _ => 0 },
            FilterRow::ClearArtists if self.enabled(row) => self.artists.clear(),
            FilterRow::ClearArtists => {}
            FilterRow::Owned => self.owned = !self.owned,
            FilterRow::ShowHidden => self.show_hidden = !self.show_hidden,
            FilterRow::Heading(_) | FilterRow::From | FilterRow::To | FilterRow::AddGenre | FilterRow::Explore | FilterRow::AddArtist | FilterRow::Apply => {}
        }
    }

    /// h / l on a year row; false when the row has nothing to adjust.
    fn adjust(&mut self, row: FilterRow, delta: i32) -> bool {
        // up to this year: my charts can show the year in progress
        let this_year = chrono::Datelike::year(&chrono::Local::now());
        let clamp = |y: i32| y.clamp(1959, this_year);
        match row {
            FilterRow::From => self.from = clamp(self.from + delta),
            FilterRow::To => self.to = clamp(self.to + delta),
            FilterRow::Mode => self.by_range = !self.by_range,
            FilterRow::Source => self.source = self.source.next(delta),
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
    /// arguments of the last run that succeeded, taken by the next render
    ok_args: Option<Vec<String>>,
}

/// Tokens of a genre spec as `hits` reads them (`genre_filter`): a token runs to a comma or to whitespace
/// followed by `+`/`-`, so "+hip hop -country" is two genres. Lowercased, like the matching in `hits`.
fn parse_genre_spec(spec: &str) -> Vec<(i8, String)> {
    let mut tokens = Vec::new();
    for part in spec.split(',') {
        let mut current = String::new();
        let words: Vec<&str> = part.split_whitespace().collect();
        for (n, word) in words.iter().enumerate() {
            if n > 0 && (word.starts_with('+') || word.starts_with('-')) {
                tokens.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        tokens.push(current);
    }
    tokens
        .into_iter()
        .filter_map(|tok| {
            let (sign, name) = match tok.chars().next()? {
                '-' => (-1, &tok[1..]),
                '+' => (1, &tok[1..]),
                _ => (1, tok.as_str()),
            };
            let name = name.trim().to_lowercase();
            (!name.is_empty()).then_some((sign, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use crate::tests::fixtures::ctx;
    use rmpc_mpd::commands::Song;

    fn row(file: Option<&str>, rank: u32) -> HitsRow {
        serde_json::from_value(serde_json::json!({
            "rank": rank, "pct": 1, "cohort": 3, "artist": "Test", "title": "Song",
            "year": 1980, "file": file,
        })).unwrap()
    }

    fn pane() -> HitsPane {
        let mut pane = HitsPane::new("unused.json".to_owned(), Vec::new());
        pane.rows = vec![row(None, 1), row(Some("other.flac"), 2), row(Some("current.flac"), 3)];
        pane.all_rows = pane.rows.clone();
        pane.state.set_content_and_viewport_len(pane.rows.len(), 2);
        pane.state.select(Some(1), 0);
        pane.focus_filters = true;
        pane
    }

    #[rstest]
    fn jump_matches_song_id_then_file_and_preserves_query(mut ctx: Ctx) {
        ctx.queue = vec![
            Song { id: 7, file: "other.flac".to_owned(), ..Default::default() },
            Song { id: 8, file: "current.flac".to_owned(), ..Default::default() },
            Song { id: 9, file: "current.flac".to_owned(), ..Default::default() },
        ];
        ctx.status.songid = Some(9);
        let mut pane = pane();
        pane.query = "current".to_owned();
        assert!(pane.jump_to_current(&ctx));
        assert_eq!(pane.state.get_selected(), Some(2));
        assert!(!pane.focus_filters);
        assert_eq!(pane.query, "current");
        assert_eq!(ctx.status.songid, Some(9));
        assert!(pane.jump_to_current(&ctx)); // second press centers without changing the match
        assert_eq!(pane.state.get_selected(), Some(2));
        pane.rows.push(row(Some("current.flac"), 4));
        pane.state.set_content_and_viewport_len(4, 2);
        pane.state.select(Some(3), 0);
        assert!(pane.jump_to_current(&ctx));
        assert_eq!(pane.state.get_selected(), Some(3), "keep a selected duplicate");
    }

    #[rstest]
    fn jump_missing_or_filtered_song_leaves_selection_and_focus(mut ctx: Ctx) {
        ctx.queue = vec![Song { id: 9, file: "current.flac".to_owned(), ..Default::default() }];
        let mut pane = pane();
        for songid in [None, Some(99)] {
            ctx.status.songid = songid;
            assert!(!pane.jump_to_current(&ctx));
        }
        ctx.status.songid = Some(9);
        pane.rows.pop(); // the playing song remains in all_rows, but not in the visible result
        pane.query = "other".to_owned();
        assert!(!pane.jump_to_current(&ctx));
        assert_eq!(pane.state.get_selected(), Some(1));
        assert!(pane.focus_filters);
        assert_eq!(pane.query, "other");
        pane.rows.clear();
        assert!(!pane.jump_to_current(&ctx));
    }

    #[rstest]
    fn queue_action_routing_does_not_swallow_common_or_global(mut ctx: Ctx) {
        let mut pane = pane();
        pane.filters = Some(Filters::default());
        pane.filter_sel = pane.filter_rows().len() - 1; // Right on Apply focuses the table
        let mut event = ActionEvent::from(Arc::new(vec![
            QueueActions::Delete.into(), CommonAction::Right.into(),
        ]));
        pane.handle_action(&mut event, &mut ctx).unwrap();
        assert!(!pane.focus_filters, "the common filter navigation still handles the key");
        let mut event = ActionEvent::from(Arc::new(vec![
            QueueActions::DeleteAll.into(),
            crate::config::keys::GlobalAction::TogglePause.into(),
        ]));
        pane.handle_action(&mut event, &mut ctx).unwrap();
        assert!(event.claim_global().is_some());
        let mut event = ActionEvent::from(Arc::new(vec![
            QueueActions::JumpToCurrent.into(),
            crate::config::keys::GlobalAction::TogglePause.into(),
        ]));
        pane.handle_action(&mut event, &mut ctx).unwrap();
        assert!(event.claim_global().is_none(), "an absent match must not trigger playback");
    }

    #[test]
    fn clear_rows_are_disabled_with_nothing_to_clear() {
        let mut f = Filters::default();
        f.genres.iter_mut().for_each(|g| g.1 = 0);
        f.artists = vec![("ABBA".to_owned(), 0)];
        for row in [FilterRow::ClearGenres, FilterRow::ClearArtists] {
            assert!(!f.enabled(row) && !f.hoverable(row));
        }
        f.toggle(FilterRow::ClearArtists);
        assert_eq!(f.artists.len(), 1, "a disabled clear keeps the rows");
        f.genres[0].1 = -1;
        f.artists[0].1 = 1;
        assert!(f.enabled(FilterRow::ClearGenres) && f.hoverable(FilterRow::ClearGenres));
        f.toggle(FilterRow::ClearArtists);
        assert!(f.artists.is_empty());
        assert!(!f.hoverable(FilterRow::Heading("Genres")) && !f.hoverable(FilterRow::Apply));
    }

    #[test]
    fn genre_spec_round_trip() {
        let mut f = Filters::default();
        f.add_genres("+hip hop -country, italo-disco");
        assert_eq!(f.genre_spec(), "+hip hop, -country, +italo-disco");
        let back = Filters::from_args(&HitsArgs { genre: Some(f.genre_spec()), ..HitsArgs::default() });
        assert_eq!(back.genre_spec(), f.genre_spec());
        assert_eq!(back.genres.len(), GENRES.len() + 1);
    }

    #[test]
    fn exclusion_only_genres_stay_one_argument() {
        let mut f = Filters::default();
        f.add_genres("-country");
        let args = f.args("out.json");
        assert!(args.contains(&"--genre=-country".to_owned()));
        assert!(!args.contains(&"-g".to_owned()));
    }

    #[test]
    fn genres_heading_counts_and_clear() {
        let mut f = Filters::default();
        assert_eq!(f.genres_heading(), "Genres: all");
        f.add_genres("-country");
        assert_eq!(f.genres_heading(), "Genres: -1");
        f.add_genres("+rock, +pop, italo-disco");
        assert_eq!(f.genres_heading(), "Genres: +3 -1");
        f.toggle(FilterRow::ClearGenres);
        assert_eq!(f.genres_heading(), "Genres: all");
        assert_eq!(f.genre_spec(), "");
    }

    #[test]
    fn artists_round_trip_through_args() {
        let mut f = Filters::default();
        assert_eq!(f.artists_heading(), "Artists: all");
        f.add_artist("Earth, Wind & Fire", 1);
        f.add_artist("Madonna", -1);
        assert_eq!(f.artists_heading(), "Artists: +1 -1");
        let args = f.args("x.json");
        let spec = args.iter().find_map(|a| a.strip_prefix("--artist=")).unwrap().to_owned();
        assert_eq!(spec, "+Earth, Wind & Fire; -Madonna");
        let back = Filters::from_args(&HitsArgs { artist: Some(spec), ..Default::default() });
        assert_eq!(back.artists, vec![("Earth, Wind & Fire".to_owned(), 1), ("Madonna".to_owned(), -1)]);
        let mut cleared = back;
        cleared.toggle(FilterRow::ClearArtists);
        assert!(cleared.artists.is_empty() && !cleared.args("x").iter().any(|a| a.starts_with("--artist")));
    }

    #[test]
    fn genre_spec_tokens() {
        assert_eq!(parse_genre_spec("-thrash metal"), vec![(-1, "thrash metal".to_owned())]);
        assert_eq!(parse_genre_spec("rock -country"), vec![(1, "rock".to_owned()), (-1, "country".to_owned())]);
        assert_eq!(parse_genre_spec(" , +Synth-Pop,"), vec![(1, "synth-pop".to_owned())]);
    }
}
