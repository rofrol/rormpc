//! rormpc: Hits pane. A table of the ranked chart hits written by the `hits` CLI (`hits ... --json PATH`) with
//! a details panel for the selected row. Rows are chart entries, not directories: a missing song is a dimmed
//! row with nothing to play. Enter / double click play the selected owned song (its queue entry if it is
//! already queued, else appended), `a` appends without playing unless already queued; the queue is never
//! replaced. The ranking itself stays in `hits`: the filter
//! column on the left (h/l moves between it and the table) runs `hits --json` in a background thread on Apply.
//! Missing songs can be fetched through `hits fetch` (a verified import queue): the menu queues them and starts
//! the worker, and each missing row shows its state from the queue file. "Downloads" in the filter column lists
//! every download waiting for review or failed, across all results, with the chart song beside what was
//! downloaded; a preview plays outside MPD (the Versions pane's player) and stops before any decision.

use std::{
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::SystemTime,
};

use anyhow::{Context, Result};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Color, Modifier, Style},
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
            menu::{list_section::ListSection, modal::MenuModal},
        },
        rormpc_actions,
        rormpc_exceptions::{self, Kind, RowException},
        rormpc_hits_rules::{self, RankBy, SETS, YearsOf},
        rormpc_preview::{self, Preview},
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
    /// the selection's counts (`hits` 0.2.38+), shown after the rule formula under the filters
    #[serde(default)]
    counts: Option<HitsCounts>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
struct HitsCounts {
    /// rows in the result
    selected: u32,
    /// songs the set chips leave before the period, genre, artist, Top % and owned filters
    candidates: u32,
    /// rows shown because of a pin, and rows an exclusion took out (`hits` with exceptions)
    #[serde(default)]
    pinned: u32,
    #[serde(default)]
    excluded: u32,
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
    /// excluded songs kept in the result, marked (`--show-excluded`, the old `--show-hidden`)
    #[serde(default)]
    show_excluded: bool,
    /// the old shorthand (files before the set chips): mapped onto sets, rank and years of
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    sort: Option<String>,
    /// the set chips, e.g. `["+billboard", "-likes"]`; absent in files before them
    #[serde(default)]
    sets: Option<Vec<String>>,
    #[serde(default)]
    rank: Option<String>,
    #[serde(default)]
    years_of: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // flags of the JSON row
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
    /// hidden with `hits hide` (an exclusion scoped to Billboard); present only with --show-excluded
    #[serde(default)]
    hidden: bool,
    /// an applicable pin ✚ / exclusion ⊘ (exceptions to the rules, `hits except`)
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    excluded: bool,
    /// every exception on the song, applying or not, for the details
    #[serde(default)]
    exceptions: Vec<RowException>,
    /// the chart song's key (`main artist|title`): what an exclusion of a missing row is keyed by
    #[serde(default)]
    chart_key: Option<String>,
    /// why a recommendation is there ("similar to …")
    #[serde(default)]
    reason: Option<String>,
    /// false: outside the rank's population (or Rank by none); `rank` is then only a unique row number
    #[serde(default = "ranked_default")]
    ranked: bool,
}

fn ranked_default() -> bool {
    true
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
    #[serde(default)]
    year: Option<i32>,
    #[serde(default)]
    url: Option<String>,
    /// how the download was identified (acoustid, mb-url, ...)
    #[serde(default)]
    method: Option<String>,
    #[serde(default)]
    file: Option<String>,
    /// the rest comes only from `hits fetch status --json` (the Downloads view)
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    video_title: Option<String>,
    /// seconds of the chart recording on `MusicBrainz`
    #[serde(default)]
    chart_length: Option<u32>,
    #[serde(default)]
    staged: Option<Staged>,
    /// the staged file's artist is the uploader channel (a download without a `MusicBrainz` match is tagged from
    /// `YouTube`)
    #[serde(default)]
    tags_from_channel: bool,
}

/// The staged file of an item in review, as `hits fetch status --json` read it.
#[derive(Debug, Clone, Default, Deserialize)]
struct Staged {
    #[serde(default)]
    exists: bool,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
    /// measured seconds
    #[serde(default)]
    length: Option<u32>,
}

impl FetchItem {
    fn in_downloads(&self) -> bool {
        matches!(self.state.as_str(), "review" | "failed")
    }

    fn busy(&self) -> bool {
        matches!(self.state.as_str(), "queued" | "searching" | "downloading" | "verifying")
    }
}

/// State shared with the thread that runs `hits fetch status --json` for the Downloads view.
#[derive(Debug, Default)]
struct DownloadsJob {
    running: bool,
    /// the queue changed during a run: read it once more
    again: bool,
    ready: Option<std::result::Result<Vec<FetchItem>, String>>,
}

/// More than this between the chart recording and the download is highlighted (`hits fetch`'s `DURATION_SLACK`).
const LENGTH_SLACK: i64 = 5;

fn mmss(s: u32) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

/// The downloaded length with its difference to the chart's ("3:45 (-1 s)"), and whether it is off by more than
/// the slack the fetch allows.
fn length_cell(expected: Option<u32>, got: Option<u32>) -> (String, bool) {
    match (expected, got) {
        (_, None) => ("?".to_owned(), false),
        (None, Some(g)) => (mmss(g), false),
        (Some(e), Some(g)) => {
            let diff = i64::from(g) - i64::from(e);
            let text = if diff == 0 { mmss(g) } else { format!("{} ({diff:+} s)", mmss(g)) };
            (text, diff.abs() > LENGTH_SLACK)
        }
    }
}

/// A few words for the Downloads table's "Why" column.
fn short_reason(f: &FetchItem) -> &'static str {
    let text = f.reason.as_deref().or(f.error.as_deref()).unwrap_or_default();
    match f.state.as_str() {
        "review" if text.starts_with("no MusicBrainz match") => "no MB match",
        "review" if text.starts_with("other recording") => "other recording",
        "review" if text.starts_with("different song") => "different song",
        "review" => "to review",
        _ if text.contains("rejected before") => "all rejected",
        _ if text.starts_with("no YouTube upload") => "no upload fits",
        _ if text.contains("429") => "YouTube 429",
        _ => "failed",
    }
}

/// Why an item is in review or failed, in plain words.
fn plain_reason(f: &FetchItem) -> String {
    let reason = f.reason.as_deref().unwrap_or_default();
    let error = f.error.as_deref().unwrap_or_default();
    if f.state == "review" {
        if reason.starts_with("no MusicBrainz match") {
            return "MusicBrainz knows no recording for this upload, so its tags came from YouTube (the uploader and \
                    the video title). Listen to it: if it is the chart song, accept it as the chart song."
                .to_owned();
        }
        if reason.starts_with("other recording") {
            return "MusicBrainz says this is another recording of the same song (a re-recording, a live or an \
                    album version), not the one that charted."
                .to_owned();
        }
        if let Some(what) = reason.strip_prefix("different song: ") {
            return format!("MusicBrainz identifies the upload as a different song: {what}.");
        }
        return reason.to_owned();
    }
    if error.contains("rejected before") {
        return format!("Every YouTube candidate was rejected before ({error}).");
    }
    if let Some(rest) = error.strip_prefix("no YouTube upload within ") {
        return format!(
            "No YouTube upload fits: none within {rest}. Retry later (search results change), or hide the song."
        );
    }
    if error.contains("429") {
        return "YouTube refused more requests (429): retry in an hour.".to_owned();
    }
    format!("The fetch failed: {error}")
}

/// Open a URL in the default browser, in the background.
fn open_url(url: &str) {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    match Command::new(opener).arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(mut child) => {
            status_info!("Opened {url}");
            std::thread::spawn(move || child.wait()); // reap it
        }
        Err(err) => status_error!("{}", crate::shared::dependencies::cannot_run(opener, &err)),
    }
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

/// The Next cell and the row style of a Hits row. The playing (or paused) song shows `▶0` and is painted like the
/// Queue's playing row, also when hidden (the paint replaces DIM); the ▶ keeps it visible under the cursor style.
fn next_and_style(next: Option<String>, playing: bool, owned: bool, hidden: bool, highlighted: Style) -> (String, Style) {
    match (playing, owned && !hidden) {
        (true, _) => ("▶0".to_owned(), highlighted),
        (false, true) => (next.unwrap_or_default(), Style::default()),
        (false, false) => (next.unwrap_or_default(), Style::default().add_modifier(Modifier::DIM)),
    }
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
    /// the rule formula with the result's counts, between the filter rows and Apply
    summary_area: Rect,
    apply_area: Rect,
    /// counts of the result on screen (`hits` 0.2.38+)
    counts: Option<HitsCounts>,
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
    /// the Downloads view (its row in the filter column): review and failed fetch items instead of the chart rows
    downloads: bool,
    dl_items: Vec<FetchItem>,
    dl_state: DirState<TableState>,
    /// the queue file's mtime the shown Downloads were read for; `dl_read` false reads them on the next render
    dl_mtime: Option<SystemTime>,
    dl_read: bool,
    dl_job: Arc<Mutex<DownloadsJob>>,
    dl_error: Option<String>,
    preview: Arc<Mutex<Preview>>,
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
            summary_area: Rect::default(),
            apply_area: Rect::default(),
            counts: None,
            applied_args: None,
            genre_input: Arc::new(Mutex::new(None)),
            cohort_artists: Vec::new(),
            artist_input: Arc::new(Mutex::new(None)),
            explorer: Arc::new(Mutex::new(Explorer::default())),
            fetch: Vec::new(),
            fetch_mtime: None,
            downloads: false,
            dl_items: Vec::new(),
            dl_state: DirState::default(),
            dl_mtime: None,
            dl_read: false,
            dl_job: Arc::new(Mutex::new(DownloadsJob::default())),
            dl_error: None,
            preview: Arc::new(Mutex::new(Preview::default())),
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
                self.counts = file.counts;
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
        let missing = self.rows.iter().filter(|x| x.file.is_none() && !x.hidden && !x.excluded).count();
        let item = self.fetch_for(r).cloned();
        let busy = self.fetch.iter().any(FetchItem::busy);
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
                Some("review" | "failed") => pane.decision_items(&mut section, item.as_ref().expect("fetch item")),
                Some("rejected") => {
                    let f = item.clone().expect("rejected item");
                    let a = pane.clone();
                    section.add_item("Retry fetching this song (another upload)", move |ctx| {
                        a.fetch_decision(ctx, "retry", &[], f.key.clone());
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
        FetchHandle { job: Arc::clone(&self.job), command: self.command.clone(), preview: Arc::clone(&self.preview) }
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
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::Apply => {
                self.leave_downloads();
                self.apply(ctx);
            }
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::Downloads => {
                if self.downloads {
                    self.leave_downloads();
                } else {
                    self.enter_downloads();
                }
                return true;
            }
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
            CommonAction::Confirm | CommonAction::Select if row == FilterRow::Exceptions => {
                rormpc_exceptions::open_list(ctx, &self.command, Some(&self.rerun_done(ctx)));
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
                    // every liked song with it, ranked by plays as before the set chips
                    filters.sets = [0, 1, 0, 0];
                    filters.rank = RankBy::Plays;
                    filters.years_of = Some(YearsOf::Release);
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
                let text = match row {
                    FilterRow::Downloads => self.downloads_label(),
                    FilterRow::ShowExcluded => self.show_excluded_label(filters),
                    _ => filters.line(*row),
                };
                let label_style = if *row == FilterRow::Downloads && self.downloads {
                    ctx.config.theme.preview_label_style.add_modifier(Modifier::BOLD) // the view is open
                } else if matches!(
                    row,
                    FilterRow::Heading(_) | FilterRow::RankBy | FilterRow::YearsOf | FilterRow::Mode | FilterRow::Apply
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
        let width = usize::from(self.summary_area.width.saturating_sub(1)).max(1);
        let summary: Vec<Line> = rormpc_hits_rules::wrap(&self.summary(filters), width)
            .into_iter()
            .map(|l| Line::from(vec![Span::raw(" "), Span::styled(l, Style::default().add_modifier(Modifier::DIM))]))
            .collect();
        frame.render_widget(Paragraph::new(summary), self.summary_area);
        frame.render_widget(Paragraph::new(self.apply_line(filters, apply_idx, ctx)), self.apply_area);
    }

    /// The rule formula of the filters, with the counts of the result on screen when it was made by them:
    /// "(Billboard ∪ Likes) − Recommended ∩ 1980-1989 ∩ rock · 1,204 of 8,312 · +2 pinned · 1 excluded" (as
    /// `hits_rules.summary`: rows a pin added are not in the first number).
    fn summary(&self, filters: &Filters) -> String {
        let text = filters.formula();
        let applied = self.applied_args.as_ref() == Some(&filters.args(&self.path.to_string_lossy()));
        match (applied, self.counts) {
            (true, Some(c)) => rormpc_hits_rules::summary(&text, c.selected, c.candidates, c.pinned, c.excluded),
            _ => text,
        }
    }

    /// "  [ ] show excluded (3)": how many rows exceptions took out of the result on screen.
    fn show_excluded_label(&self, filters: &Filters) -> String {
        let line = filters.line(FilterRow::ShowExcluded);
        match self.counts.map(|c| c.excluded) {
            Some(n) if n > 0 => format!("{line} ({n})"),
            _ => line,
        }
    }

    /// " · ✚ 2 · ⊘ 1" in the status line when exceptions changed the result.
    fn exceptions_summary(&self) -> String {
        let Some(c) = self.counts else { return String::new() };
        let mut out = String::new();
        if c.pinned > 0 {
            out = format!(" · {} {} pinned", Kind::Pin.mark(), c.pinned);
        }
        if c.excluded > 0 {
            out = format!("{out} · {} {} excluded", Kind::Exclude.mark(), c.excluded);
        }
        out
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
        if r.file.is_none() && !r.hidden && !r.excluded {
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
        let (pin, exclude) = (self.except_menu(ctx, Kind::Pin, &r), self.except_menu(ctx, Kind::Exclude, &r));
        let (list_command, list_done) = (self.command.clone(), self.rerun_done(ctx));
        let (pin_hint, exclude_hint) = (rormpc_exceptions::key_hint(ctx, Kind::Pin), rormpc_exceptions::key_hint(ctx, Kind::Exclude));
        let owned_row = r.file.is_some();
        menu = menu.list_section(ctx, move |mut section| {
            if owned_row {
                section.add_item(format!("Pin in results…{pin_hint}"), move |ctx| {
                    pin(ctx);
                    Ok(())
                });
            }
            section.add_item(format!("Exclude from results…{exclude_hint}"), move |ctx| {
                exclude(ctx);
                Ok(())
            });
            section.add_item("Exceptions… (every pin and exclusion)", move |ctx| {
                rormpc_exceptions::open_list(ctx, &list_command, Some(&list_done));
                Ok(())
            });
            Some(section)
        });
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
            if r.ranked {
                field("Rank", format!("#{} of {} ({:.0}%)", r.rank, r.cohort, r.pct.ceil()))
            } else {
                field("Rank", "— (not in the rank's population)".to_owned())
            },
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
                        lines.push(Line::from(Span::styled(
                            "menu: Preview, Accept as the chart song, Reject, … (all of them: Downloads, top left)",
                            dim,
                        )));
                    }
                } else {
                    lines.push(Line::default());
                    lines.push(Line::from(Span::styled("menu: Fetch this song", dim)));
                }
            }
        }
        if !r.exceptions.is_empty() {
            lines.push(Line::default());
            for e in &r.exceptions {
                let mark = if e.action == "pin" { Kind::Pin.mark() } else { Kind::Exclude.mark() };
                let style = if e.applies { Style::default() } else { dim };
                lines.push(Line::from(vec![
                    Span::styled("Exception: ", key),
                    Span::styled(format!("{mark} {}", rormpc_exceptions::describe(e)), style),
                ]));
            }
        } else if r.hidden {
            lines.push(Line::from(Span::styled("hidden from Hits (menu: Unhide)", dim)));
        }
        lines.push(Line::from(Span::styled("+ pin · - exclude (asks the scope) · menu: Exceptions…", dim)));
        lines
    }

    /// After an exception was recorded or removed: run `hits` again, so the marks and rows follow.
    fn rerun_done(&self, ctx: &Ctx) -> rormpc_exceptions::Done {
        let (job, sender) = (Arc::clone(&self.job), ctx.app_event_sender.clone());
        Arc::new(move || {
            job.lock().expect("hits job lock").rerun = true;
            let _ = sender.send(AppEvent::RequestRender);
        })
    }

    /// `+` / `-` on a row (or its menu item): the scope menu for that row, offering the `+` sets of the filters.
    fn except_menu(&self, ctx: &Ctx, kind: Kind, r: &HitsRow) -> impl FnOnce(&Ctx) + 'static {
        let target = rormpc_exceptions::Target {
            file: r.file.clone(),
            chart_key: r.chart_key.clone(),
            artist: r.artist.clone(),
            title: r.title.clone(),
        };
        let plus = self.filters.as_ref().map_or([0; 4], |f| f.sets.map(|s| s.max(0)));
        let (command, done) = (self.command.clone(), self.rerun_done(ctx));
        move |ctx: &Ctx| rormpc_exceptions::open_scope_menu(ctx, command, kind, target, plus, Some(done))
    }

    /// `+` / `-` in the filter column: include / exclude a set, genre or artist row (pressed again: off).
    fn sign_filter_row(&mut self, sign: i8) {
        let rows = self.filter_rows();
        let (Some(&row), Some(filters)) = (rows.get(self.filter_sel), self.filters.as_mut()) else { return };
        let flip = |s: &mut i8| *s = if *s == sign { 0 } else { sign };
        match row {
            FilterRow::Set(i) => flip(&mut filters.sets[i]),
            FilterRow::Genre(i) => flip(&mut filters.genres[i].1),
            FilterRow::Artist(i) => flip(&mut filters.artists[i].1),
            _ => status_info!("+ / - set a set, genre or artist row here; on a song row they pin / exclude it"),
        }
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
    /// "Downloads (2 to review)", else "Downloads (3 failed)": the filter column's entry, counted over the whole
    /// queue (every result), not only the rows on screen.
    fn downloads_label(&self) -> String {
        let review = self.fetch.iter().filter(|f| f.state == "review").count();
        let failed = self.fetch.iter().filter(|f| f.state == "failed").count();
        match (review, failed) {
            (0, 0) => "Downloads".to_owned(),
            (0, n) => format!("Downloads ({n} failed)"),
            (n, _) => format!("Downloads ({n} to review)"),
        }
    }

    fn enter_downloads(&mut self) {
        self.downloads = true;
        self.dl_read = false; // read the status now, also when the queue did not change
        self.focus_filters = false;
    }

    /// Back to the chart; a preview stops with the view.
    fn leave_downloads(&mut self) {
        if self.downloads {
            self.downloads = false;
            self.preview.lock().expect("preview lock").stop();
        }
    }

    /// `hits fetch status --json` in the background when the queue file changed since the list on screen was read
    /// (it adds the staged file's tags and length, and the chart length). One run at a time; a change during a run
    /// reads once more after it.
    fn load_downloads(&mut self, ctx: &Ctx) {
        if self.dl_read && self.dl_mtime == self.fetch_mtime {
            return;
        }
        self.dl_read = true;
        self.dl_mtime = self.fetch_mtime;
        let mut job = self.dl_job.lock().expect("downloads lock");
        if job.running {
            job.again = true;
            return;
        }
        job.running = true;
        drop(job);
        let (job, command, sender) = (Arc::clone(&self.dl_job), self.command.clone(), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            loop {
                let out = Command::new(&command[0]).args(&command[1..]).args(["fetch", "status", "--json"]).output();
                let result = match out {
                    Ok(o) if o.status.success() => serde_json::from_slice::<FetchQueue>(&o.stdout)
                        .map(|q| q.items)
                        .map_err(|e| format!("hits fetch status: {e}")),
                    Ok(o) => Err(format!("hits fetch status: {}", last_line(&o.stderr, "failed"))),
                    Err(err) => Err(crate::shared::dependencies::cannot_run(&command[0], &err)),
                };
                let mut j = job.lock().expect("downloads lock");
                j.ready = Some(result);
                if !std::mem::take(&mut j.again) {
                    j.running = false;
                    break;
                }
            }
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// Show a fresh status: review first, then failed, each in queue order; the cursor stays on the same song,
    /// else at the same place.
    fn set_downloads(&mut self, items: Vec<FetchItem>) {
        let keep = self.selected_download().map(|f| f.key.clone());
        let old = self.dl_state.get_selected().unwrap_or(0);
        let (mut list, failed): (Vec<_>, Vec<_>) =
            items.into_iter().filter(FetchItem::in_downloads).partition(|f| f.state == "review");
        list.extend(failed);
        self.dl_items = list;
        self.dl_state.set_content_and_viewport_len(self.dl_items.len(), self.state_viewport());
        let idx = keep
            .and_then(|k| self.dl_items.iter().position(|f| f.key == k))
            .unwrap_or_else(|| old.min(self.dl_items.len().saturating_sub(1)));
        self.dl_state.select((!self.dl_items.is_empty()).then_some(idx), 0);
    }

    fn selected_download(&self) -> Option<&FetchItem> {
        self.dl_state.get_selected().and_then(|i| self.dl_items.get(i))
    }

    fn open_download_menu(&self, ctx: &Ctx) {
        let Some(f) = self.selected_download().cloned() else { return };
        let pane = self.clone_for_fetch();
        let busy = self.fetch.iter().any(FetchItem::busy);
        let mut menu = MenuModal::new(ctx).list_section(ctx, |mut section| {
            pane.decision_items(&mut section, &f);
            Some(section)
        });
        if busy {
            let command = self.command.clone();
            menu = menu.list_section(ctx, move |section| {
                Some(section.item("Stop fetching after the current song", move |_| {
                    let _ = Command::new(&command[0]).args(&command[1..]).args(["fetch", "cancel"]).status();
                    Ok(())
                }))
            });
        }
        let menu = menu.list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(())))).build();
        modal!(ctx, menu);
    }

    /// The Downloads view: the list where the chart table was, the comparison where its details were.
    fn render_downloads(&mut self, frame: &mut Frame, main: Rect, details: Rect, ctx: &Ctx) {
        let [table_area, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(main);
        self.table_area = table_area;
        self.dl_state.set_content_and_viewport_len(self.dl_items.len(), self.state_viewport());
        let dim = Style::default().add_modifier(Modifier::DIM);
        let loading = self.dl_job.lock().expect("downloads lock").running;
        if self.dl_items.is_empty() {
            let text =
                if loading { "Reading the fetch queue…" } else { "Nothing waits for review and nothing failed." };
            frame.render_widget(Paragraph::new(Line::from(Span::styled(format!(" {text}"), dim))), table_area);
        } else {
            let rows = self.dl_items.iter().map(|f| {
                Row::new(vec![
                    Cell::from(fetch_mark(&f.state)),
                    Cell::from(f.artist.clone()),
                    Cell::from(f.title.clone()),
                    Cell::from(f.year.filter(|y| *y > 0).map(|y| y.to_string()).unwrap_or_default()),
                    Cell::from(Span::styled(short_reason(f), dim)),
                ])
            });
            let header = Row::new(["", "Artist", "Title", "Year", "Why"]).style(ctx.config.theme.preview_label_style);
            let table = Table::new(rows, [
                Constraint::Length(1),
                Constraint::Percentage(35),
                Constraint::Percentage(65),
                Constraint::Length(4),
                Constraint::Length(15),
            ])
            .header(header)
            .column_spacing(1)
            .style(ctx.config.as_text_style())
            .row_highlight_style(ctx.config.theme.current_item_style);
            frame.render_stateful_widget(table, table_area, self.dl_state.as_render_state_ref());
        }
        let count = |states: &[&str]| self.fetch.iter().filter(|f| states.contains(&f.state.as_str())).count();
        let (queued, fetching) = (count(&["queued"]), count(&["searching", "downloading", "verifying"]));
        let mut status = format!(" Downloads · {} to review · {} failed", count(&["review"]), count(&["failed"]));
        if queued + fetching > 0 {
            status = format!("{status} · {queued} queued, {fetching} fetching");
        }
        let status = match &self.dl_error {
            Some(err) => Span::styled(format!(" {err}"), Style::default().add_modifier(Modifier::BOLD)),
            None => Span::styled(status, dim),
        };
        // a playing preview takes the hint's place, so it is seen however narrow the pane
        let mut preview = self.preview.lock().expect("preview lock");
        let hint = if preview.playing() {
            Span::styled(
                format!(" ▶ preview: {} · Enter: Stop the preview", preview.what),
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(" Enter: preview, accept, reject, another candidate · Esc: back to the chart", dim)
        };
        drop(preview);
        frame.render_widget(Paragraph::new(vec![Line::from(status), Line::from(hint)]), footer);
        self.render_download_details(frame, details, ctx);
    }

    /// Expected (the chart) beside downloaded (the staged file): a length off by more than the fetch's slack is
    /// highlighted, and "uploader ≠ artist" says the tags came from the `YouTube` channel.
    fn render_download_details(&self, frame: &mut Frame, area: Rect, ctx: &Ctx) {
        let Some(f) = self.selected_download() else {
            frame.render_widget(Paragraph::new("No download selected."), area);
            return;
        };
        let key = ctx.config.theme.preview_label_style;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let warn = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
        let field =
            |name: &str, value: String| Line::from(vec![Span::styled(format!("{name}: "), key), Span::raw(value)]);
        let [head, compare, rest] =
            Layout::vertical([Constraint::Length(3), Constraint::Length(4), Constraint::Min(0)]).areas(area);
        let year = f.year.filter(|y| *y > 0).map(|y| format!(" · {y}")).unwrap_or_default();
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(f.title.clone(), bold)),
                Line::from(format!("{}{year}", f.artist)),
            ]),
            head,
        );
        let staged = f.staged.clone().filter(|s| s.exists && f.state == "review");
        let none = || Cell::from(Span::styled("—", dim));
        let (got_artist, got_title, got_length) = match &staged {
            Some(s) => {
                let (length, off) = length_cell(f.chart_length, s.length);
                let artist_style = if f.tags_from_channel { warn } else { Style::default() };
                (
                    Cell::from(Span::styled(s.artist.clone(), artist_style)),
                    Cell::from(s.title.clone()),
                    Cell::from(Span::styled(length, if off { warn } else { Style::default() })),
                )
            }
            None => (none(), none(), none()),
        };
        let expected_length = f.chart_length.map_or_else(|| "?".to_owned(), mmss);
        let rows = vec![
            Row::new(vec![Cell::from("Artist"), Cell::from(f.artist.clone()), got_artist]),
            Row::new(vec![Cell::from("Title"), Cell::from(f.title.clone()), got_title]),
            Row::new(vec![Cell::from("Length"), Cell::from(expected_length), got_length]),
        ];
        let table = Table::new(rows, [Constraint::Length(6), Constraint::Percentage(50), Constraint::Percentage(50)])
            .header(Row::new(["", "Expected", "Downloaded"]).style(key))
            .column_spacing(1)
            .style(ctx.config.as_text_style());
        frame.render_widget(table, compare);

        let mut lines = Vec::new();
        if f.state == "review" && staged.is_none() && f.staged.is_some() {
            lines.push(Line::from(Span::styled("The staged file is gone: Accept fails; try another candidate.", warn)));
        }
        if f.tags_from_channel && staged.is_some() {
            lines.push(Line::from(Span::styled("uploader ≠ artist: the tags came from the YouTube channel", warn)));
        }
        lines.push(Line::default());
        lines.push(field("Why", plain_reason(f)));
        if let Some(channel) = f.channel.as_deref().filter(|c| !c.is_empty()) {
            let video = f.video_title.as_deref().unwrap_or_default();
            let upload = if video.is_empty() { channel.to_owned() } else { format!("{channel} · {video}") };
            lines.push(field("Upload", upload));
        }
        if let Some(url) = &f.url {
            lines.push(field("URL", url.clone()));
        }
        if let Some(method) = f.method.as_deref().filter(|m| !m.is_empty()) {
            lines.push(field("Matched by", method.to_owned()));
        }
        if let Some(found) = f.found.as_deref().filter(|_| f.state == "review" && !f.tags_from_channel) {
            lines.push(field("Identified as", found.to_owned()));
        }
        if let Some(file) = f.file.as_deref().filter(|_| f.state == "review") {
            lines.push(Line::from(Span::styled(file.to_owned(), dim)));
        }
        lines.push(Line::default());
        let actions: &[&str] = match (f.state.as_str(), f.url.is_some()) {
            ("review", _) => &["Preview", "Open on YouTube", "Accept as the chart song", "Accept with current tags",
                "Reject", "Try another candidate"],
            (_, true) => &["Open on YouTube", "Try another candidate", "Retry"],
            (_, false) => &["Retry"],
        };
        lines.push(Line::from(Span::styled(format!("Enter: {}", actions.join(" · ")), dim)));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rest);
    }

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
    preview: Arc<Mutex<Preview>>,
}

impl FetchHandle {
    /// `hits fetch add` with these arguments, then the worker (`hits fetch run`; a second one exits at once),
    /// in a background thread. When songs arrived, `hits` runs again so they show as owned.
    fn start_fetch(&self, ctx: &Ctx, add: Vec<String>) {
        let (job, command, sender) = (Arc::clone(&self.job), self.command.clone(), ctx.app_event_sender.clone());
        let mut args = vec!["fetch".to_owned()];
        args.extend(add);
        std::thread::spawn(move || {
            if args.len() > 1 {
                match Command::new(&command[0]).args(&command[1..]).args(&args).output() {
                    Ok(out) if out.status.success() => status_info!("{}", last_line(&out.stdout, "queued")),
                    Ok(out) => return status_error!("hits fetch: {}", last_line(&out.stderr, "failed")),
                    Err(err) => return status_error!("{}", crate::shared::dependencies::cannot_run(&command[0], &err)),
                }
            }
            run_worker(&job, &command);
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// One decision on a queued song (`hits fetch VERB [EXTRA] KEY`), with the preview stopped first (accept moves
    /// the file, reject and another delete it). Then accept reruns `hits`, retry and another start the worker,
    /// after the decision is written, so the worker finds the item queued.
    fn fetch_decision(&self, ctx: &Ctx, verb: &'static str, extra: &'static [&'static str], key: String) {
        self.preview.lock().expect("preview lock").stop();
        let (job, command, sender) = (Arc::clone(&self.job), self.command.clone(), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let out =
                Command::new(&command[0]).args(&command[1..]).args(["fetch", verb]).args(extra).arg(&key).output();
            match out {
                Ok(out) if out.status.success() => match verb {
                    "accept" => job.lock().expect("hits job lock").rerun = true,
                    "retry" | "another" => {
                        let _ = sender.send(AppEvent::RequestRender);
                        run_worker(&job, &command);
                    }
                    _ => {}
                },
                Ok(out) => status_error!("hits fetch {verb}: {}", last_line(&out.stderr, "failed")),
                Err(err) => status_error!("{}", crate::shared::dependencies::cannot_run(&command[0], &err)),
            }
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// The decisions on a download in review or a failed one: the chart row's menu and the Downloads view share
    /// them. Preview / Stop play the staged file outside MPD; nothing plays until asked.
    fn decision_items(&self, section: &mut ListSection, f: &FetchItem) {
        let staged = f.file.clone().filter(|file| f.state == "review" && std::path::Path::new(file).is_file());
        if let Some(file) = staged {
            let (preview, what) = (Arc::clone(&self.preview), f.title.clone());
            section.add_item("Preview the download", move |_| {
                rormpc_preview::start(&preview, std::path::Path::new(&file), 0, format!("{what} (download)"));
                Ok(())
            });
            if self.preview.lock().expect("preview lock").playing() {
                let preview = Arc::clone(&self.preview);
                section.add_item("Stop the preview", move |_| {
                    preview.lock().expect("preview lock").stop();
                    Ok(())
                });
            }
        }
        if let Some(url) = f.url.clone() {
            section.add_item("Open on YouTube", move |_| {
                open_url(&url);
                Ok(())
            });
        }
        if f.state == "review" {
            let (a, b, k1, k2) = (self.clone(), self.clone(), f.key.clone(), f.key.clone());
            section.add_item("Accept as the chart song (chart artist and title)", move |ctx| {
                a.fetch_decision(ctx, "accept", &["--as-chart"], k1);
                Ok(())
            });
            section.add_item("Accept with current tags", move |ctx| {
                b.fetch_decision(ctx, "accept", &[], k2);
                Ok(())
            });
            let (c, key, title) = (self.clone(), f.key.clone(), f.title.clone());
            section.add_item("Reject the download…", move |ctx| {
                c.confirm_reject(ctx, key, &title);
                Ok(())
            });
        }
        if f.url.is_some() {
            let (a, key) = (self.clone(), f.key.clone());
            section.add_item("Try another candidate (skip this upload)", move |ctx| {
                a.fetch_decision(ctx, "another", &[], key);
                Ok(())
            });
        }
        if f.state == "failed" {
            let (a, key) = (self.clone(), f.key.clone());
            section.add_item("Retry fetching this song", move |ctx| {
                a.fetch_decision(ctx, "retry", &[], key);
                Ok(())
            });
        }
    }

    /// Reject is durable (the song is never fetched again), so it asks first and says how to undo it.
    fn confirm_reject(&self, ctx: &Ctx, key: String, title: &str) {
        let message = vec![format!(
            "Reject the download of '{title}'?\n\nThe staged file is deleted and the song is not fetched again. \
             Retry (the song's menu in Hits) undoes it and fetches another upload, skipping this one."
        )];
        let pane = self.clone();
        let go = move |ctx: &Ctx| -> Result<()> {
            pane.fetch_decision(ctx, "reject", &[], key.clone());
            Ok(())
        };
        modal!(
            ctx,
            ConfirmModal::builder()
                .ctx(ctx)
                .message(message)
                .action(Action::CustomButtons {
                    buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), ("Reject", Box::new(go))],
                })
                .build()
        );
    }
}

/// `hits fetch run` until the queue is done (a second worker exits at once); reruns `hits` when songs arrived.
fn run_worker(job: &Mutex<Job>, command: &[String]) {
    match Command::new(&command[0]).args(&command[1..]).args(["fetch", "run"]).output() {
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

impl Drop for HitsPane {
    fn drop(&mut self) {
        self.preview.lock().expect("preview lock").stop(); // never leave a player running after rormpc quits
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
        // Apply gets its row first, so even a tiny pane shows it; the rule summary sits above it, never scrolled
        let summary_lines = self.filters.as_ref().map_or(0, |f| {
            let width = usize::from(filter_area.width.saturating_sub(1)).max(1);
            rormpc_hits_rules::wrap(&self.summary(f), width).len().min(4) as u16
        });
        let [list_area, summary_area, apply_area] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(summary_lines), Constraint::Length(1)])
                .areas(filter_area);
        self.summary_area = summary_area;
        if list_area != self.filter_area {
            self.hover_filter = None; // resized: the row under the pointer is another one now
        }
        self.filter_area = list_area;
        self.apply_area = apply_area;
        self.scroll_filters(0); // the pane may have been resized
        self.reload_fetch();
        self.render_filters(frame, list_area, ctx);
        if self.downloads {
            self.load_downloads(ctx);
            let ready = self.dl_job.lock().expect("downloads lock").ready.take();
            match ready {
                Some(Ok(items)) => {
                    self.dl_error = None;
                    self.set_downloads(items);
                }
                Some(Err(err)) => self.dl_error = Some(err),
                None => {}
            }
            // the comparison needs room: the details take more of the width than the chart's
            let [_, main, details] =
                Layout::horizontal([Constraint::Length(24), Constraint::Min(40), Constraint::Percentage(40)])
                    .spacing(2)
                    .areas(area);
            self.render_downloads(frame, main, details, ctx);
            return Ok(());
        }
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
        // the song the Queue paints (playing or paused, not stopped): every row with its file lights up
        let playing_file = ctx.current_song().map(|s| s.file.as_str());
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
            let playing = r.file.is_some() && r.file.as_deref() == playing_file;
            let (next, row_style) =
                next_and_style(next, playing, owned, r.hidden || r.excluded, ctx.config.theme.highlighted_item_style);
            // a row outside the rank's population (or with Rank by none) has no rank to show
            let (rank, pct) = if r.ranked {
                (format!("#{}", r.rank), format!("{:.0}%", r.pct.ceil()))
            } else {
                ("—".to_owned(), String::new())
            };
            Row::new(vec![
                Cell::from(rank),
                Cell::from(pct),
                Cell::from(if r.excluded {
                    Kind::Exclude.mark()
                } else if r.pinned {
                    Kind::Pin.mark()
                } else if r.hidden {
                    "h"
                } else if owned {
                    "✓"
                } else {
                    missing_mark
                }),
                like_cell,
                Cell::from(next),
                Cell::from(r.artist.clone()),
                Cell::from(r.title.clone()),
                Cell::from(if r.year > 0 { r.year.to_string() } else { String::new() }),
                Cell::from(if owned && r.plays > 0 { r.plays.to_string() } else { String::new() }),
                Cell::from(Span::styled(state, dim)),
            ])
            .style(row_style)
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
                    " {} · {} hits · {} in library{} · updated {}{}",
                    self.label,
                    self.rows.len(),
                    owned,
                    self.exceptions_summary(),
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

    fn on_hide(&mut self, _ctx: &Ctx) -> Result<()> {
        self.preview.lock().expect("preview lock").stop(); // a preview never outlives the view it was started in
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
                self.leave_downloads();
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
        if self.downloads {
            match event.kind {
                MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                    self.focus_filters = false;
                    if let Some(idx) = self.dl_state.get_at_rendered_row(row) {
                        self.dl_state.select(Some(idx), ctx.config.scrolloff);
                        if matches!(event.kind, MouseEventKind::DoubleClick) {
                            self.open_download_menu(ctx);
                        }
                    }
                }
                MouseEventKind::ScrollUp => self.dl_state.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff),
                MouseEventKind::ScrollDown => self.dl_state.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff),
                _ => return Ok(()),
            }
            ctx.render()?;
            return Ok(());
        }
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
        if let Some(action) = event.claim_queue().cloned() {
            match action {
                QueueActions::JumpToCurrent => {
                    if !self.downloads && self.jump_to_current(ctx) {
                        ctx.render()?;
                    }
                    return Ok(()); // recognized even without a match: never fall through to playback
                }
                // + / -: pin / exclude the row (scope menu); in the filter column, include / exclude a row there
                QueueActions::PinSong | QueueActions::ExcludeSong if !self.downloads => {
                    let pin = matches!(action, QueueActions::PinSong);
                    if self.focus_filters {
                        self.sign_filter_row(if pin { 1 } else { -1 });
                    } else if let Some(r) = self.selected().cloned() {
                        self.except_menu(ctx, if pin { Kind::Pin } else { Kind::Exclude }, &r)(ctx);
                    }
                    ctx.render()?;
                    return Ok(());
                }
                _ => event.abandon(), // Queue-only actions must not consume common or global bindings
            }
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
        if self.downloads {
            match action {
                CommonAction::Left => self.focus_filters = true,
                // Enter never plays here: it opens the decisions, Preview among them
                CommonAction::ContextMenu | CommonAction::Confirm => self.open_download_menu(ctx),
                CommonAction::Down => self.dl_state.next(scrolloff, wrap),
                CommonAction::Up => self.dl_state.prev(scrolloff, wrap),
                CommonAction::DownHalf => self.dl_state.next_half_viewport(scrolloff),
                CommonAction::UpHalf => self.dl_state.prev_half_viewport(scrolloff),
                CommonAction::PageDown => self.dl_state.next_viewport(scrolloff),
                CommonAction::PageUp => self.dl_state.prev_viewport(scrolloff),
                CommonAction::Top => self.dl_state.first(),
                CommonAction::Bottom => self.dl_state.last(),
                CommonAction::Close => self.leave_downloads(),
                _ => {
                    event.abandon();
                    return Ok(());
                }
            }
            ctx.render()?;
            return Ok(());
        }
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
    /// the Downloads view: every fetch item in review or failed (its label counts them, see `downloads_label`)
    Downloads,
    /// a set chip (`rormpc_hits_rules::SETS`): + include / - exclude / off, like a genre
    Set(usize),
    RankBy,
    /// the period's axis; shows the default that follows Rank by
    YearsOf,
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
    /// excluded songs back as dim rows with ⊘ (the pane adds the count of the result on screen)
    ShowExcluded,
    /// the Exceptions list: every pin and exclusion, Enter removes one
    Exceptions,
    Apply,
}

/// What the filter column edits; turned into `hits` arguments on Apply.
#[derive(Debug, Clone)]
struct Filters {
    /// the fixed set chips, in `SETS` order: -1 exclude, 0 off, 1 include
    sets: [i8; 4],
    rank: RankBy,
    /// None: follows Rank by (`RankBy::default_years`)
    years_of: Option<YearsOf>,
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
    show_excluded: bool,
}

impl Default for Filters {
    fn default() -> Self {
        let mut decades = [false; 8];
        decades[3] = true; // 1980s
        Self { sets: [1, 0, 0, 0], rank: RankBy::Billboard, years_of: None, by_range: false, decades, from: 1985, to: 1992, tops: [true, false, false], genres: pinned_genres().into_iter().map(|g| (g, 0)).collect(), artists: Vec::new(), owned: false, show_excluded: false }
    }
}

impl Filters {
    fn rows(&self) -> Vec<FilterRow> {
        let mut rows = vec![FilterRow::Downloads, FilterRow::Heading("Sets")];
        rows.extend((0..SETS.len()).map(FilterRow::Set));
        rows.extend([FilterRow::RankBy, FilterRow::YearsOf]);
        if self.has_years() {
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
        rows.extend([FilterRow::Owned, FilterRow::ShowExcluded, FilterRow::Exceptions, FilterRow::Apply]);
        rows
    }

    fn effective_years_of(&self) -> YearsOf {
        self.years_of.unwrap_or(self.rank.default_years())
    }

    /// Recommendations alone have no year to filter on: no period rows, no --years.
    fn has_years(&self) -> bool {
        self.sets != [0, 0, 0, 1]
    }

    /// The period as `--years`, or None for every year: the Billboard chart years need a period (a decade by
    /// default), any other axis without a ticked decade means all years.
    fn period(&self) -> Option<String> {
        let chart_years = self.rank == RankBy::Billboard && self.effective_years_of() == YearsOf::Chart;
        let all_years = !chart_years
            && !self.by_range
            && !self.decades.iter().any(|d| *d);
        (self.has_years() && !all_years).then(|| self.years())
    }

    /// The Top % ranges as `--top`, or None: no box ticked, or Rank by none (no rank to cut).
    fn top(&self) -> Option<String> {
        let tops: Vec<String> =
            TOPS.iter().zip(self.tops).filter(|(_, on)| *on).map(|((lo, hi), _)| format!("{lo}-{hi}")).collect();
        (self.rank != RankBy::None && !tops.is_empty()).then(|| tops.join(","))
    }

    /// The formula of these filters, as `hits` prints it.
    fn formula(&self) -> String {
        rormpc_hits_rules::formula(&rormpc_hits_rules::Rules {
            sets: self.sets,
            period: self.period(),
            top: self.top(),
            genres: &self.genres,
            artists: &self.artists,
            owned: self.owned,
        })
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
        let mut args: Vec<String> = SETS
            .iter()
            .zip(self.sets)
            .filter(|(_, sign)| *sign != 0)
            // one token: argparse takes a separate value starting with '-' ("-likes") for an option
            .map(|((key, _, _), sign)| format!("--set={}{key}", if sign > 0 { '+' } else { '-' }))
            .collect();
        args.extend(["--rank".to_owned(), self.rank.arg().to_owned()]);
        args.extend(["--years-of".to_owned(), self.effective_years_of().arg().to_owned()]);
        if let Some(period) = self.period() {
            args.extend(["--years".to_owned(), period]);
        }
        // no Top % (or no rank): every row, the unranked ones after the ranked
        match self.top() {
            Some(top) => args.extend(["--top".to_owned(), top]),
            None => args.extend(["-n".to_owned(), "0".to_owned()]),
        }
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
        if self.show_excluded {
            args.push("--show-excluded".to_owned());
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
        f.show_excluded = args.show_excluded || args.show_hidden;
        // files before the set chips name an old source, mapped as `hits --source` maps it
        let (sets, rank, years_of) = rormpc_hits_rules::from_source(args.source.as_deref(), args.sort.as_deref());
        f.sets = args.sets.as_deref().map_or(sets, rormpc_hits_rules::parse_sets);
        f.rank = args.rank.as_deref().and_then(RankBy::parse).filter(|_| args.sets.is_some()).unwrap_or(rank);
        let years_of = args.years_of.as_deref().and_then(YearsOf::parse).unwrap_or(years_of);
        // the default that follows Rank by stays "auto", so changing Rank by moves it along
        f.years_of = (years_of != f.rank.default_years()).then_some(years_of);
        if args.period.is_none() {
            f.decades = [false; 8]; // all years
        }
        if f.rank == RankBy::None || args.top.as_deref() == Some("1-100") {
            f.tops = [false; 3];
        }
        f
    }

    fn line(&self, row: FilterRow) -> String {
        let check = |on: bool| if on { "[x]" } else { "[ ]" };
        match row {
            FilterRow::Downloads => "Downloads".to_owned(), // the pane draws it with its counts
            FilterRow::Set(i) => {
                let mark = match self.sets[i] { 1 => "+", -1 => "-", _ => " " };
                format!("  [{mark}] {}", SETS[i].1)
            }
            FilterRow::RankBy => format!("Rank by:  ‹{}›", self.rank.label()),
            // the default follows Rank by: shown as "auto" so it is never guessed
            FilterRow::YearsOf => match self.years_of {
                None => format!("Years of: ‹{}› auto", self.rank.default_years().arg()),
                Some(y) => format!("Years of: ‹{}›", y.arg()),
            },
            // listening years read differently from the songs' release or chart years
            FilterRow::Mode => format!(
                "{} {}",
                if self.effective_years_of() == YearsOf::Listened { "Listened:" } else { "Period:" },
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
            FilterRow::ShowExcluded => format!("  {} show excluded", check(self.show_excluded)),
            FilterRow::Exceptions => "  ⋯ exceptions…".to_owned(),
            FilterRow::Apply => "  [ Apply ]".to_owned(),
        }
    }

    /// False for "× clear genres" / "× clear artists" when no box is + or -: drawn dim, Enter does nothing.
    fn enabled(&self, row: FilterRow) -> bool {
        match row {
            FilterRow::ClearGenres => self.genres.iter().any(|(_, s)| *s != 0),
            FilterRow::ClearArtists => self.artists.iter().any(|(_, s)| *s != 0),
            // Top % needs a rank
            FilterRow::Top(_) => self.rank != RankBy::None,
            _ => true,
        }
    }

    /// Rows that underline under the mouse: actions and the label of a checkbox, not headings, the ‹…› cyclers,
    /// Apply (its own button look) or a disabled row.
    fn hoverable(&self, row: FilterRow) -> bool {
        self.enabled(row)
            && matches!(
                row,
                FilterRow::Downloads
                    | FilterRow::ClearGenres
                    | FilterRow::ClearArtists
                    | FilterRow::AddGenre
                    | FilterRow::Explore
                    | FilterRow::AddArtist
                    | FilterRow::Decade(_)
                    | FilterRow::Set(_)
                    | FilterRow::Top(_)
                    | FilterRow::Genre(_)
                    | FilterRow::Artist(_)
                    | FilterRow::Owned
                    | FilterRow::ShowExcluded
                    | FilterRow::Exceptions
            )
    }

    /// Space / Enter on a row.
    fn toggle(&mut self, row: FilterRow) {
        match row {
            FilterRow::Set(i) => self.sets[i] = rormpc_hits_rules::next_sign(self.sets[i]),
            FilterRow::RankBy => self.rank = self.rank.next(1),
            FilterRow::YearsOf => self.years_of = YearsOf::next(self.years_of, 1),
            FilterRow::Mode => self.by_range = !self.by_range,
            FilterRow::Decade(i) => self.decades[i] = !self.decades[i],
            FilterRow::Top(i) if self.enabled(row) => self.tops[i] = !self.tops[i],
            FilterRow::Top(_) => {}
            FilterRow::Genre(i) => self.genres[i].1 = match self.genres[i].1 { 0 => 1, 1 => -1, _ => 0 },
            FilterRow::ClearGenres => self.genres.iter_mut().for_each(|(_, s)| *s = 0),
            FilterRow::Artist(i) => self.artists[i].1 = match self.artists[i].1 { 0 => 1, 1 => -1, _ => 0 },
            FilterRow::ClearArtists if self.enabled(row) => self.artists.clear(),
            FilterRow::ClearArtists => {}
            FilterRow::Owned => self.owned = !self.owned,
            FilterRow::ShowExcluded => self.show_excluded = !self.show_excluded,
            FilterRow::Heading(_) | FilterRow::Exceptions | FilterRow::Downloads | FilterRow::From | FilterRow::To | FilterRow::AddGenre | FilterRow::Explore | FilterRow::AddArtist | FilterRow::Apply => {}
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
            FilterRow::RankBy => self.rank = self.rank.next(delta),
            FilterRow::YearsOf => self.years_of = YearsOf::next(self.years_of, delta),
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

    #[test]
    fn playing_row_gets_the_glyph_and_the_highlight_even_when_hidden() {
        let hl = Style::default().add_modifier(Modifier::BOLD);
        let dim = Style::default().add_modifier(Modifier::DIM);
        assert_eq!(next_and_style(Some("0".to_owned()), true, true, false, hl), ("▶0".to_owned(), hl));
        // a hidden owned row plays: the paint replaces DIM
        assert_eq!(next_and_style(Some("0".to_owned()), true, true, true, hl), ("▶0".to_owned(), hl));
        assert_eq!(next_and_style(Some("↑10".to_owned()), false, true, false, hl), ("↑10".to_owned(), Style::default()));
        assert_eq!(next_and_style(Some("-2".to_owned()), false, true, true, hl), ("-2".to_owned(), dim));
        assert_eq!(next_and_style(None, false, false, false, hl), (String::new(), dim));
        // the Next column is 4 cells wide: `▶0` fits as `↑10` and `-10` do
        for m in ["▶0", "↑10", "-10"] {
            assert!(unicode_width::UnicodeWidthStr::width(m) <= 4);
        }
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

    #[rstest]
    fn plus_and_minus_set_a_filter_row_and_leave_other_keys_alone(mut ctx: Ctx) {
        let mut pane = pane();
        pane.filters = Some(Filters::default());
        pane.filter_sel = pane.filter_rows().iter().position(|r| *r == FilterRow::Set(1)).unwrap();
        for (action, want) in [(QueueActions::ExcludeSong, -1), (QueueActions::PinSong, 1), (QueueActions::PinSong, 0)] {
            let mut event = ActionEvent::from(Arc::new(vec![action.into()]));
            pane.handle_action(&mut event, &mut ctx).unwrap();
            assert_eq!(pane.filters.as_ref().unwrap().sets[1], want);
        }
        pane.filter_sel = pane.filter_rows().iter().position(|r| matches!(r, FilterRow::Genre(_))).unwrap();
        let mut event = ActionEvent::from(Arc::new(vec![QueueActions::ExcludeSong.into()]));
        pane.handle_action(&mut event, &mut ctx).unwrap();
        assert_eq!(pane.filters.as_ref().unwrap().genres[0].1, -1);
    }

    #[test]
    fn show_excluded_round_trips_and_reads_the_old_show_hidden() {
        let mut f = Filters::default();
        f.show_excluded = true;
        let args = f.args("out.json");
        assert!(args.contains(&"--show-excluded".to_owned()));
        let old: HitsArgs = serde_json::from_str(r#"{"period": "1980-1989", "show_hidden": true}"#).unwrap();
        assert!(Filters::from_args(&old).show_excluded);
        let rows = Filters::default().rows();
        let at = rows.iter().position(|r| *r == FilterRow::ShowExcluded).unwrap();
        assert_eq!(&rows[at..], [FilterRow::ShowExcluded, FilterRow::Exceptions, FilterRow::Apply]);
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

    /// The `args` object `hits` writes into its JSON for these command-line arguments.
    fn written_args(args: &[String]) -> HitsArgs {
        let value = |flag: &str| args.iter().position(|a| a == flag).map(|i| args[i + 1].clone());
        HitsArgs {
            period: value("--years"),
            top: value("--top"),
            rank: value("--rank"),
            years_of: value("--years-of"),
            sets: Some(args.iter().filter_map(|a| a.strip_prefix("--set=").map(str::to_owned)).collect()),
            ..HitsArgs::default()
        }
    }

    #[test]
    fn set_chips_rank_and_years_round_trip() {
        let mut f = Filters::default();
        assert_eq!(f.args("x.json")[..4], ["--set=+billboard", "--rank", "billboard", "--years-of"]);
        f.toggle(FilterRow::Set(1)); // likes: off -> +
        f.toggle(FilterRow::Set(3));
        f.toggle(FilterRow::Set(3)); // recommended: off -> + -> -
        f.adjust(FilterRow::RankBy, 1); // Billboard -> my plays
        assert_eq!(f.line(FilterRow::YearsOf), "Years of: ‹listened› auto");
        f.decades = [false; 8];
        let args = f.args("x.json");
        assert_eq!(args[..7], ["--set=+billboard", "--set=+likes", "--set=-recommended", "--rank", "plays", "--years-of", "listened"]);
        assert!(!args.contains(&"--years".to_owned())); // no decade: every listening year
        let back = Filters::from_args(&written_args(&args));
        assert_eq!((back.sets, back.rank, back.years_of, back.decades), (f.sets, RankBy::Plays, None, [false; 8]));
        assert_eq!(f.formula(), "(Billboard ∪ Likes) − Recommended ∩ Top 1-10%");
        // an explicit axis stays explicit, the default one stays "auto"
        f.toggle(FilterRow::YearsOf);
        assert_eq!(f.line(FilterRow::YearsOf), "Years of: ‹release›");
        let back = Filters::from_args(&written_args(&f.args("x.json")));
        assert_eq!(back.years_of, Some(YearsOf::Release));
    }

    #[test]
    fn rank_none_disables_top_and_takes_every_row() {
        let mut f = Filters { sets: [0, 0, 0, 1], rank: RankBy::None, ..Filters::default() };
        assert!(!f.enabled(FilterRow::Top(0)));
        f.toggle(FilterRow::Top(1));
        assert_eq!(f.tops, [true, false, false]); // the disabled row does not change
        let args = f.args("x.json");
        assert!(!args.contains(&"--top".to_owned()) && args.windows(2).any(|w| w == ["-n", "0"]));
        // recommendations alone have no year: no period rows, no --years
        assert!(!f.rows().contains(&FilterRow::Mode) && !args.contains(&"--years".to_owned()));
        assert_eq!(f.formula(), "Recommended");
    }

    #[test]
    fn files_before_the_set_chips_load_from_their_source() {
        let back = Filters::from_args(&HitsArgs {
            source: Some("playlists".to_owned()),
            sort: Some("rediscover".to_owned()),
            rank: Some("chart".to_owned()), // the old default value, not a choice
            top: Some("1-100".to_owned()),
            ..HitsArgs::default()
        });
        assert_eq!((back.sets, back.rank, back.years_of, back.decades), ([0, 0, 1, 0], RankBy::Rediscover, None, [false; 8]));
        assert_eq!(back.tops, [false; 3]); // "1-100" was "no box ticked"
        let mine = Filters::from_args(&HitsArgs { source: Some("mine".to_owned()), ..HitsArgs::default() });
        assert_eq!((mine.sets, mine.rank, mine.years_of), ([0; 4], RankBy::Plays, None));
        assert_eq!(mine.line(FilterRow::Mode), "Listened: ‹decades›");
        let charts = Filters::from_args(&HitsArgs { period: Some("1980-1989".to_owned()), ..HitsArgs::default() });
        assert_eq!((charts.sets, charts.rank, charts.years_of), ([1, 0, 0, 0], RankBy::Billboard, None));
    }

    /// The JSON `hits --json` writes, as rormpc-tools' `tests/test_rormpc_contract.py` checks it on its side.
    #[test]
    fn hits_file_contract() {
        let text = r#"{"version": 1, "generated_at": "2026-10-09T23:00:00", "label": "Hits 1980-1989 · x",
            "artists": [{"name": "Toto", "songs": 2}],
            "args": {"period": "1980-1989", "top": "1-10", "genre": "+rock", "artist": "", "owned": false,
                     "rank": "billboard", "years_of": "chart", "sets": ["+billboard", "-likes"],
                     "show_hidden": false, "source": null, "sort": "plays"},
            "rules": {"schema": 1, "sets": {"billboard": 1, "likes": -1}, "rank": "billboard",
                      "years_of": "chart", "period": "1980-1989", "top": "1-10", "genre": "+rock", "artist": "",
                      "owned": false},
            "formula": "Billboard − Likes ∩ 1980-1989 ∩ Top 1-10% ∩ rock",
            "summary": "Billboard − Likes ∩ 1980-1989 ∩ Top 1-10% ∩ rock · 1 of 995",
            "counts": {"selected": 1, "owned": 1, "candidates": 995, "cohort": 995, "pinned": 1, "excluded": 2},
            "rank_note": "best year-end chart position within the chosen years",
            "rows": [{"rank": 3, "pct": 0.3, "cohort": 995, "ranked": true, "artist": "Toto", "title": "Africa",
                      "year": 1983, "years": [1983], "points": 98, "peak": 98, "listens": 5, "sets": ["billboard"],
                      "genres": ["rock"], "mbid": null, "file": "a.mp3", "hidden": false, "plays": 4,
                      "reason": null, "pinned": true, "excluded": false, "song_id": "id-a",
                      "chart_key": "toto|africa",
                      "exceptions": [{"id": "e1", "action": "pin", "scope": "library", "applies": true, "via": null},
                                     {"id": "hide:toto|africa", "action": "exclude", "scope": "set:likes",
                                      "applies": false, "via": "hide"}]}]}"#;
        let file: HitsFile = serde_json::from_str(text).unwrap();
        let counts = file.counts.unwrap();
        assert_eq!((counts.selected, counts.candidates, file.rows[0].ranked), (1, 995, true));
        assert_eq!((counts.pinned, counts.excluded), (1, 2));
        let r = &file.rows[0];
        assert_eq!((r.pinned, r.excluded, r.chart_key.as_deref(), r.exceptions.len()), (true, false, Some("toto|africa"), 2));
        assert!(r.exceptions[0].applies && !r.exceptions[1].applies);
        let f = Filters::from_args(&file.args);
        assert_eq!((f.sets, f.rank, f.years_of), ([1, -1, 0, 0], RankBy::Billboard, None));
        // the pane's own formula for these filters is the text `hits` wrote
        assert_eq!(f.formula(), "Billboard − Likes ∩ 1980-1989 ∩ Top 1-10% ∩ rock");
        // a row of an older file is ranked
        assert!(row(Some("a.mp3"), 1).ranked);
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

    fn item(key: &str, state: &str) -> FetchItem {
        serde_json::from_value(serde_json::json!({
            "key": key, "artist": "Captain & Tennille", "title": "Love Will Keep Us Together", "year": 1975,
            "state": state, "reason": "no MusicBrainz match", "url": "https://www.youtube.com/watch?v=vid1",
            "file": "/staging/x.mp3", "channel": "some uploader", "video_title": "a video", "chart_length": 226,
            "staged": {"exists": true, "artist": "some uploader", "title": "a video", "length": 225, "comment": ""},
            "tags_from_channel": true, "rejected_ids": ["old"],
        }))
        .expect("a fetch item")
    }

    #[test]
    fn length_difference_beyond_the_fetch_slack_is_highlighted() {
        assert_eq!(length_cell(Some(226), Some(225)), ("3:45 (-1 s)".to_owned(), false));
        assert_eq!(length_cell(Some(213), Some(248)), ("4:08 (+35 s)".to_owned(), true));
        assert_eq!(length_cell(Some(268), Some(268)), ("4:28".to_owned(), false));
        assert_eq!(length_cell(None, Some(10)), ("0:10".to_owned(), false));
        assert_eq!(length_cell(Some(10), None), ("?".to_owned(), false));
    }

    #[test]
    fn reasons_in_plain_words() {
        let mut f = item("k", "review");
        assert_eq!(short_reason(&f), "no MB match");
        assert!(plain_reason(&f).contains("tags came from YouTube"));
        f.reason = Some("different song: ABBA - Waterloo".to_owned());
        assert_eq!(short_reason(&f), "different song");
        assert!(plain_reason(&f).ends_with("a different song: ABBA - Waterloo."));
        f.state = "failed".to_owned();
        f.reason = None;
        f.error = Some("no other YouTube candidate: all 3 were rejected before".to_owned());
        assert_eq!(short_reason(&f), "all rejected");
        f.error = Some("no YouTube upload within 5 s of the chart recording (219 s) without live words".to_owned());
        assert_eq!(short_reason(&f), "no upload fits");
        assert!(plain_reason(&f).starts_with("No YouTube upload fits: none within 5 s of the chart recording (219 s)"));
    }

    #[test]
    fn downloads_list_review_then_failed_and_keep_the_cursor_by_key() {
        let mut pane = pane();
        let all = |order: &[(&str, &str)]| order.iter().map(|(k, s)| item(k, s)).collect::<Vec<_>>();
        pane.set_downloads(all(&[("a", "failed"), ("b", "review"), ("c", "ok"), ("d", "review"), ("e", "rejected")]));
        let keys: Vec<&str> = pane.dl_items.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["b", "d", "a"]);
        pane.dl_state.select(Some(1), 0); // d
        pane.set_downloads(all(&[("x", "review"), ("a", "failed"), ("b", "review"), ("d", "review")]));
        assert_eq!(pane.selected_download().map(|f| f.key.as_str()), Some("d"), "the same song after a reload");
        pane.set_downloads(all(&[("x", "review"), ("a", "failed")]));
        assert_eq!(pane.dl_state.get_selected(), Some(1), "d is gone: the same place, clamped");
        pane.set_downloads(Vec::new());
        assert_eq!(pane.selected_download().map(|f| f.key.clone()), None);
    }

    #[rstest]
    fn downloads_entry_counts_the_whole_queue_and_leaving_it_stops_the_view(ctx: Ctx) {
        let mut pane = pane();
        assert_eq!(pane.downloads_label(), "Downloads");
        pane.fetch = vec![item("a", "failed")];
        assert_eq!(pane.downloads_label(), "Downloads (1 failed)");
        pane.fetch.extend([item("b", "review"), item("c", "review"), item("d", "queued")]);
        assert_eq!(pane.downloads_label(), "Downloads (2 to review)");
        let f = Filters::default();
        assert_eq!(f.rows()[0], FilterRow::Downloads);
        assert!(f.hoverable(FilterRow::Downloads));
        pane.filters = Some(f);
        pane.filter_sel = 0;
        assert!(pane.filter_action(&CommonAction::Confirm, &ctx));
        assert!(pane.downloads && !pane.focus_filters && !pane.dl_read);
        pane.leave_downloads();
        assert!(!pane.downloads);
    }

    #[test]
    fn genre_spec_tokens() {
        assert_eq!(parse_genre_spec("-thrash metal"), vec![(-1, "thrash metal".to_owned())]);
        assert_eq!(parse_genre_spec("rock -country"), vec![(1, "rock".to_owned()), (-1, "country".to_owned())]);
        assert_eq!(parse_genre_spec(" , +Synth-Pop,"), vec![(1, "synth-pop".to_owned())]);
    }
}
