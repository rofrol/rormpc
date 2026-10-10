//! rormpc: Live playlists pane. Public playlists followed by `liveplaylist` (rormpc-tools): left the
//! subscriptions, right the selected one's items with their decision (pending / accepted / rejected) and job
//! (queued / downloading / needs match / ready / failed). The context menu (Enter) adds a playlist URL, checks for
//! new tracks, accepts or rejects items, accepts every pending item, downloads the queue and cancels it. All of it
//! runs `liveplaylist ... --json` with argv in background threads; the download worker reports progress on stderr
//! and in its status file, and SIGTERM cancels it (the CLI queues the item again).
//! Space marks items: accept (`a`, the menu) and reject (`D`, the menu) then act on the marked items. Play opens
//! this pane as its Live inbox and shows `badge()` in its header.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
};
use serde::Deserialize;

use super::Pane;
use crate::{
    config::keys::{CommonAction, actions::AddKind},
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        macros::{modal, status_error, status_info},
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        UiEvent,
        dirstack::DirState,
        modals::{input_modal::InputModal, menu::modal::MenuModal},
    },
};

const LIVEPLAYLIST: &str = "liveplaylist";

#[derive(Debug, Clone, Deserialize)]
struct Item {
    ytid: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    channel: String,
    decision: String,
    #[serde(default)]
    job: Option<String>,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    review: Option<Review>,
}

#[derive(Debug, Clone, Deserialize)]
struct Review {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Check {
    #[serde(default)]
    at: String,
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    partial: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Sub {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    playlist: String,
    #[serde(default)]
    last_check: Option<Check>,
    #[serde(default)]
    counts: BTreeMap<String, u32>,
    #[serde(default)]
    items: Vec<Item>,
}

impl Sub {
    fn count(&self, key: &str) -> u32 {
        self.counts.get(key).copied().unwrap_or(0)
    }

    fn pending(&self) -> Vec<String> {
        self.items.iter().filter(|it| it.active && it.decision == "pending").map(|it| it.ytid.clone()).collect()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Current {
    #[serde(default)]
    title: String,
}

/// `status.json` of the download worker.
#[derive(Debug, Clone, Default, Deserialize)]
struct Status {
    #[serde(default)]
    running: bool,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    current: Option<Current>,
    #[serde(default)]
    done: u32,
    #[serde(default)]
    failed: u32,
    #[serde(default)]
    total: u32,
    #[serde(default)]
    errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Listing {
    subscriptions: Vec<Sub>,
}

/// State shared with the background `liveplaylist` runs.
#[derive(Debug, Default)]
struct Job {
    loading: bool,
    /// a list loaded while one was running: load again when it ends
    stale: bool,
    /// a newer result to take on the next render
    subs: Option<Vec<Sub>>,
    error: Option<String>,
    /// a command finished: load again
    reload: bool,
    /// a short command running now ("Listing the playlist…")
    busy: Option<String>,
    /// pid of the download worker started from here
    worker: Option<u32>,
    /// the worker's last stderr line and its status file
    progress: Option<String>,
    status: Option<Status>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Subs,
    Items,
}

#[derive(Debug)]
pub struct LivePlaylistsPane {
    subs: Vec<Sub>,
    sub_state: DirState<TableState>,
    item_state: DirState<TableState>,
    focus: Focus,
    subs_area: Rect,
    items_area: Rect,
    job: Arc<Mutex<Job>>,
    /// marked items (ytids) of the selected playlist; a switch to another playlist clears them
    marked: BTreeSet<String>,
}

fn status_path() -> PathBuf {
    let cache = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()).map_or_else(
        || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".cache"),
        PathBuf::from,
    );
    cache.join("rormpc-tools/liveplaylist/status.json")
}

fn read_status() -> Option<Status> {
    std::fs::read(status_path()).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// `liveplaylist ARGS --json`: its stdout, or the error from its JSON or its last stderr line.
fn run(args: &[String]) -> Result<String, String> {
    match Command::new(LIVEPLAYLIST).args(args).arg("--json").stdin(Stdio::null()).output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            #[derive(Deserialize)]
            struct Failure {
                error: String,
            }
            let stdout = String::from_utf8_lossy(&out.stdout);
            let json = stdout.lines().rev().find_map(|l| serde_json::from_str::<Failure>(l).ok()).map(|f| f.error);
            Err(json.unwrap_or_else(|| {
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("liveplaylist failed")
                    .to_owned()
            }))
        }
        Err(err) => Err(crate::shared::dependencies::cannot_run(LIVEPLAYLIST, &err)),
    }
}

fn load(job: &Arc<Mutex<Job>>, ctx: &Ctx) {
    let mut j = job.lock().expect("live playlists job lock");
    if j.loading {
        j.stale = true;
        return;
    }
    j.loading = true;
    drop(j);
    let (job, sender) = (Arc::clone(job), ctx.app_event_sender.clone());
    std::thread::spawn(move || {
        loop {
            let result = run(&["list".to_owned()])
                .and_then(|out| serde_json::from_str::<Listing>(&out).map_err(|e| e.to_string()));
            let mut j = job.lock().expect("live playlists job lock");
            match result {
                Ok(listing) => {
                    j.subs = Some(listing.subscriptions);
                    j.error = None;
                }
                Err(err) => j.error = Some(err),
            }
            j.status = read_status();
            if !std::mem::take(&mut j.stale) {
                j.loading = false;
                break;
            }
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

/// Run a short command (add, check, accept --no-download, reject) in the background, report it, reload; with
/// `then_download`, start the worker afterwards unless one runs.
fn command(ctx: &Ctx, job: &Arc<Mutex<Job>>, args: Vec<String>, busy: &str, done: &'static str, then_download: bool) {
    job.lock().expect("live playlists job lock").busy = Some(busy.to_owned());
    let (job, sender) = (Arc::clone(job), ctx.app_event_sender.clone());
    let _ = sender.send(AppEvent::RequestRender);
    std::thread::spawn(move || {
        let ok = match run(&args) {
            Ok(_) => {
                status_info!("{done}");
                true
            }
            Err(err) => {
                status_error!("liveplaylist: {err}");
                false
            }
        };
        let start = {
            let mut j = job.lock().expect("live playlists job lock");
            j.busy = None;
            j.reload = true;
            ok && then_download && j.worker.is_none()
        };
        if start {
            download(&job, &sender);
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

/// Start `liveplaylist download`: one worker empties the whole queue. Its stderr lines are the progress; each
/// finished item reloads the list.
fn download(job: &Arc<Mutex<Job>>, sender: &crossbeam::channel::Sender<AppEvent>) {
    let child = Command::new(LIVEPLAYLIST)
        .args(["download", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(err) => {
            status_error!("{}", crate::shared::dependencies::cannot_run(LIVEPLAYLIST, &err));
            return;
        }
    };
    {
        let mut j = job.lock().expect("live playlists job lock");
        j.worker = Some(child.id());
        j.progress = Some("starting the download…".to_owned());
    }
    let (job, sender) = (Arc::clone(job), sender.clone());
    std::thread::spawn(move || {
        let mut last = String::new();
        if let Some(stderr) = child.stderr.take() {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let mut j = job.lock().expect("live playlists job lock");
                j.status = read_status();
                // "    ready" / "    failed: ..." ends an item: show its new state
                j.reload |= line.starts_with("    ");
                j.progress = Some(line.trim().to_owned());
                line.trim().clone_into(&mut last);
                drop(j);
                let _ = sender.send(AppEvent::RequestRender);
            }
        }
        let ok = child.wait().is_ok_and(|s| s.success());
        {
            let mut j = job.lock().expect("live playlists job lock");
            j.worker = None;
            j.progress = None;
            j.status = read_status();
            j.reload = true;
        }
        if ok {
            status_info!("{last}");
        } else {
            status_error!("{last}");
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

fn cancel(job: &Arc<Mutex<Job>>) {
    if let Some(pid) = job.lock().expect("live playlists job lock").worker {
        // SAFETY: kill(2) on the pid of our own child, which we have not reaped yet (its thread waits for it)
        unsafe {
            libc::kill(pid.cast_signed(), libc::SIGTERM);
        }
        status_info!("Cancelling the download…");
    }
}

fn add_url(ctx: &Ctx, job: &Arc<Mutex<Job>>) {
    let job = Arc::clone(job);
    modal!(
        ctx,
        InputModal::new(ctx)
            .title("Live playlist")
            .input_label("Public YouTube playlist URL:")
            .confirm_label("Add")
            .on_confirm(move |ctx, value| {
                let url = value.trim().to_owned();
                if !url.is_empty() {
                    let args = vec!["add".to_owned(), url];
                    command(ctx, &job, args, "Listing the playlist…", "Playlist added: review its items", false);
                }
                Ok(())
            })
    );
}

/// Accept items (queue only), then start the worker unless one runs.
fn accept(ctx: &Ctx, job: &Arc<Mutex<Job>>, id: &str, ytids: Vec<String>) {
    if ytids.is_empty() {
        return;
    }
    let busy = format!("Accepting {}…", ytids.len());
    let mut args = vec!["accept".to_owned(), id.to_owned()];
    args.extend(ytids);
    args.push("--no-download".to_owned());
    command(ctx, job, args, &busy, "Accepted", true);
}

fn reject(ctx: &Ctx, job: &Arc<Mutex<Job>>, id: &str, ytids: Vec<String>) {
    if ytids.is_empty() {
        return;
    }
    let busy = format!("Rejecting {}…", ytids.len());
    let mut args = vec!["reject".to_owned(), id.to_owned()];
    args.extend(ytids);
    command(ctx, job, args, &busy, "Rejected: never downloaded", false);
}

/// What accept and reject act on: the marked items in list order, else the item under the cursor.
fn targets(items: &[Item], marked: &BTreeSet<String>, cursor: Option<&Item>) -> Vec<String> {
    if marked.is_empty() {
        cursor.map(|it| vec![it.ytid.clone()]).unwrap_or_default()
    } else {
        items.iter().filter(|it| marked.contains(&it.ytid)).map(|it| it.ytid.clone()).collect()
    }
}

fn decision_mark(it: &Item) -> &'static str {
    match it.decision.as_str() {
        "accepted" => "✓",
        "rejected" => "✗",
        _ => "?",
    }
}

fn job_label(it: &Item) -> String {
    match it.job.as_deref() {
        Some("needs_match") => "needs match".to_owned(),
        Some("ready") if it.source.as_deref() == Some("library") => "in library".to_owned(),
        Some(job) => job.to_owned(),
        None if it.decision == "pending" => "to review".to_owned(),
        None => String::new(),
    }
}

impl LivePlaylistsPane {
    pub fn new() -> Self {
        Self {
            subs: Vec::new(),
            sub_state: DirState::default(),
            item_state: DirState::default(),
            focus: Focus::Subs,
            subs_area: Rect::default(),
            items_area: Rect::default(),
            job: Arc::new(Mutex::new(Job::default())),
            marked: BTreeSet::new(),
        }
    }

    /// Play's header badge: (items waiting for a decision, items queued or downloading while a download runs).
    pub fn badge(&self) -> (u32, u32) {
        let pending = self.subs.iter().map(|s| s.count("pending")).sum();
        let running = {
            let j = self.job.lock().expect("live playlists job lock");
            j.worker.is_some() || j.status.as_ref().is_some_and(|st| st.running)
        };
        let downloads = if running {
            self.subs.iter().map(|s| s.count("queued") + s.count("downloading")).sum::<u32>().max(1)
        } else {
            0
        };
        (pending, downloads)
    }

    /// The MPD playlists the followed playlists are written to (generated: Play's Browse shows them read-only).
    pub fn playlist_names(&self) -> Vec<String> {
        self.subs.iter().map(|s| s.playlist.clone()).filter(|p| !p.is_empty()).collect()
    }

    /// Take a newer listing (and load again after a command) without drawing: Play keeps its badge current
    /// while the inbox is closed.
    pub fn refresh(&mut self, ctx: &Ctx) {
        let (fresh, reload) = {
            let mut j = self.job.lock().expect("live playlists job lock");
            (j.subs.take(), std::mem::take(&mut j.reload))
        };
        if reload {
            load(&self.job, ctx);
        }
        if let Some(subs) = fresh {
            self.take(subs);
        }
    }

    /// Marks to clear (Esc clears them before it closes anything).
    pub fn has_marks(&self) -> bool {
        !self.marked.is_empty()
    }

    fn sub(&self) -> Option<&Sub> {
        self.sub_state.get_selected().and_then(|i| self.subs.get(i))
    }

    fn item(&self) -> Option<&Item> {
        self.sub().and_then(|s| self.item_state.get_selected().and_then(|i| s.items.get(i)))
    }

    fn state(&mut self) -> &mut DirState<TableState> {
        match self.focus {
            Focus::Subs => &mut self.sub_state,
            Focus::Items => &mut self.item_state,
        }
    }

    /// Take a newer listing, keeping the selected subscription and item by id.
    fn take(&mut self, subs: Vec<Sub>) {
        let keep_sub = self.sub().map(|s| s.id.clone());
        let keep_item = self.item().map(|it| it.ytid.clone());
        self.subs = subs;
        let si = keep_sub.and_then(|id| self.subs.iter().position(|s| s.id == id)).unwrap_or(0);
        self.sub_state.set_content_and_viewport_len(self.subs.len(), self.subs_area.height.saturating_sub(1).into());
        self.sub_state.select((!self.subs.is_empty()).then_some(si), 0);
        self.sync_items(keep_item);
    }

    fn sync_items(&mut self, keep: Option<String>) {
        if keep.is_none() {
            self.marked.clear(); // another playlist: its marks do not carry over
        }
        let (len, idx) = self.sub().map_or((0, None), |s| {
            (s.items.len(), keep.and_then(|y| s.items.iter().position(|it| it.ytid == y)))
        });
        self.item_state.set_content_and_viewport_len(len, self.items_area.height.saturating_sub(1).into());
        self.item_state.select((len > 0).then_some(idx.unwrap_or(0)), 0);
    }

    fn open_menu(&self, ctx: &Ctx) {
        let sub = self.sub().cloned();
        let item = if self.focus == Focus::Items { self.item().cloned() } else { None };
        let marked: Vec<String> = self.sub().map(|s| targets(&s.items, &self.marked, None)).unwrap_or_default();
        let worker = self.job.lock().expect("live playlists job lock").worker.is_some();
        let several = self.subs.len() > 1;
        let job = Arc::clone(&self.job);
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
                if let (Some(sub), false) = (&sub, marked.is_empty()) {
                    let (j, id, ytids) = (Arc::clone(&job), sub.id.clone(), marked.clone());
                    section.add_item(format!("Accept marked ({})", marked.len()), move |ctx| {
                        accept(ctx, &j, &id, ytids);
                        Ok(())
                    });
                    let (j, id, ytids) = (Arc::clone(&job), sub.id.clone(), marked.clone());
                    section.add_item(format!("Reject marked ({}, never download them)", marked.len()), move |ctx| {
                        reject(ctx, &j, &id, ytids);
                        Ok(())
                    });
                } else if let (Some(sub), Some(it)) = (&sub, &item) {
                    let label = match (it.decision.as_str(), it.job.as_deref()) {
                        (_, Some("needs_match")) => Some("Accept as it is (names only, no MBID)"),
                        (_, Some("failed")) => Some("Retry the download"),
                        // deleted from the library before: accepting again downloads it once the Deleted tab allows it
                        (_, Some("blocked")) => Some("Accept again (allowed again in the Deleted tab?)"),
                        ("pending" | "rejected", _) => Some("Accept"),
                        _ => None,
                    };
                    if let Some(label) = label {
                        let (job, id, ytid) = (Arc::clone(&job), sub.id.clone(), it.ytid.clone());
                        section.add_item(label, move |ctx| {
                            accept(ctx, &job, &id, vec![ytid]);
                            Ok(())
                        });
                    }
                    if it.decision != "rejected" {
                        let (job, id, ytid) = (Arc::clone(&job), sub.id.clone(), it.ytid.clone());
                        section.add_item("Reject (never download it)", move |ctx| {
                            reject(ctx, &job, &id, vec![ytid]);
                            Ok(())
                        });
                    }
                }
                if let Some(sub) = &sub {
                    let pending = sub.pending();
                    if !pending.is_empty() {
                        let (job, id) = (Arc::clone(&job), sub.id.clone());
                        section.add_item(format!("Accept all pending ({})", pending.len()), move |ctx| {
                            accept(ctx, &job, &id, pending);
                            Ok(())
                        });
                    }
                    let (job, id) = (Arc::clone(&job), sub.id.clone());
                    section.add_item("Check for new tracks", move |ctx| {
                        command(ctx, &job, vec!["check".to_owned(), id], "Checking the playlist…", "Checked", false);
                        Ok(())
                    });
                }
                if several {
                    let job = Arc::clone(&job);
                    section.add_item("Check all playlists", move |ctx| {
                        command(ctx, &job, vec!["check".to_owned()], "Checking the playlists…", "Checked", false);
                        Ok(())
                    });
                }
                section.add_item("Add a playlist URL…", move |ctx| {
                    add_url(ctx, &job);
                    Ok(())
                });
                Some(section)
            })
            .list_section(ctx, |mut section| {
                let queued = self.subs.iter().map(|s| s.count("queued")).sum::<u32>();
                if worker {
                    let job = Arc::clone(&self.job);
                    section.add_item("Cancel the download", move |_| {
                        cancel(&job);
                        Ok(())
                    });
                } else if queued > 0 {
                    let job = Arc::clone(&self.job);
                    section.add_item(format!("Download queued ({queued})"), move |ctx| {
                        download(&job, &ctx.app_event_sender);
                        Ok(())
                    });
                } else {
                    return None; // nothing to download or cancel: no empty section
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Close", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    fn footer(&self, ctx: &Ctx) -> Vec<Line<'static>> {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let j = self.job.lock().expect("live playlists job lock");
        let first = if let Some(err) = &j.error {
            Span::styled(format!(" liveplaylist: {err}"), bold)
        } else if let Some(busy) = &j.busy {
            Span::styled(format!(" {busy}"), dim)
        } else if let (Some(_), Some(st)) = (j.worker, &j.status) {
            let current = st.current.as_ref().map(|c| c.title.clone()).unwrap_or_default();
            Span::raw(format!(" Downloading {}/{} {current}", st.done + st.failed + 1, st.total.max(1)))
        } else if let Some(progress) = &j.progress {
            Span::styled(format!(" {progress}"), dim)
        } else if let Some(st) = j.status.as_ref().filter(|st| !st.running && !st.errors.is_empty()) {
            Span::styled(
                format!(" Last download {}: {}", st.state.as_deref().unwrap_or("done"), st.errors.join(" · ")),
                bold,
            )
        } else if self.subs.is_empty() {
            Span::styled(" No live playlists yet: Enter → Add a playlist URL…", dim)
        } else if !self.marked.is_empty() {
            Span::styled(format!(" {} marked · a accept · D reject · Esc clears the marks", self.marked.len()), dim)
        } else {
            let pending = self.subs.iter().map(|s| s.count("pending")).sum::<u32>();
            Span::styled(
                format!(" {} playlists · {pending} items to review · Space marks · Enter: menu", self.subs.len()),
                dim,
            )
        };
        drop(j);
        let key = ctx.config.theme.preview_label_style;
        let second = match (self.focus, self.item(), self.sub()) {
            (Focus::Items, Some(it), _) => {
                let (label, text) = if let Some(err) = &it.error {
                    ("Failed", err.clone())
                } else if let Some(r) = &it.review {
                    let guess = if r.title.is_empty() { String::new() } else { format!(": {} - {}", r.artist, r.title) };
                    ("Needs match", format!("{}{guess}", r.reason))
                } else if let Some(path) = &it.path {
                    ("File", path.clone())
                } else {
                    ("Video", format!("https://www.youtube.com/watch?v={}", it.ytid))
                };
                Line::from(vec![Span::styled(format!(" {label}: "), key), Span::raw(text)])
            }
            (_, _, Some(sub)) => match &sub.last_check {
                Some(c) if !c.ok => Line::from(vec![
                    Span::styled(" Last check failed (nothing changed): ", key),
                    Span::styled(c.error.clone().unwrap_or_default(), bold),
                ]),
                Some(c) => Line::from(vec![
                    Span::styled(" Checked: ", key),
                    Span::raw(c.at.replace('T', " ")),
                    Span::styled(if c.partial { " (partial listing: nothing marked gone)" } else { "" }, dim),
                    Span::styled(format!(" · MPD playlist {:?}", sub.playlist), dim),
                ]),
                None => Line::default(),
            },
            _ => Line::default(),
        };
        vec![Line::from(first), second]
    }
}

impl Pane for LivePlaylistsPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let [main, footer] = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(area);
        let [subs_area, items_area] =
            Layout::horizontal([Constraint::Percentage(30), Constraint::Min(30)]).spacing(2).areas(main);
        (self.subs_area, self.items_area) = (subs_area, items_area);
        self.refresh(ctx);
        self.sub_state.set_content_and_viewport_len(self.subs.len(), subs_area.height.saturating_sub(1).into());
        let item_len = self.sub().map_or(0, |s| s.items.len());
        self.item_state.set_content_and_viewport_len(item_len, items_area.height.saturating_sub(1).into());

        let dim = Style::default().add_modifier(Modifier::DIM);
        let label = ctx.config.theme.preview_label_style;
        let highlight = |focused: bool| if focused { ctx.config.theme.current_item_style } else { Style::default().add_modifier(Modifier::REVERSED | Modifier::DIM) };

        let rows = self.subs.iter().map(|s| {
            let failed = s.last_check.as_ref().is_some_and(|c| !c.ok);
            Row::new(vec![
                Cell::from(if failed { "!" } else { "" }),
                Cell::from(s.title.clone()),
                Cell::from(match s.count("pending") {
                    0 => String::new(),
                    n => format!("{n} new"),
                }),
            ])
        });
        let table = Table::new(rows, [Constraint::Length(1), Constraint::Min(10), Constraint::Length(8)])
            .header(Row::new(["", "Playlist", ""]).style(label))
            .column_spacing(1)
            .style(ctx.config.as_text_style())
            .row_highlight_style(highlight(self.focus == Focus::Subs));
        frame.render_stateful_widget(table, subs_area, self.sub_state.as_render_state_ref());

        let items = self.sub().map(|s| s.items.clone()).unwrap_or_default();
        let rows = items.iter().map(|it| {
            let name = if it.channel.is_empty() { it.title.clone() } else { format!("{}  · {}", it.title, it.channel) };
            Row::new(vec![
                Cell::from(if self.marked.contains(&it.ytid) { "●" } else { decision_mark(it) }),
                Cell::from(if it.active { job_label(it) } else { "gone upstream".to_owned() }),
                Cell::from(name),
            ])
            .style(if it.active && it.decision != "rejected" { Style::default() } else { dim })
        });
        let table = Table::new(rows, [Constraint::Length(1), Constraint::Length(13), Constraint::Min(10)])
            .header(Row::new(["", "State", "Item"]).style(label))
            .column_spacing(1)
            .style(ctx.config.as_text_style())
            .row_highlight_style(highlight(self.focus == Focus::Items));
        frame.render_stateful_widget(table, items_area, self.item_state.as_render_state_ref());

        frame.render_widget(Paragraph::new(self.footer(ctx)), footer);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        load(&self.job, ctx);
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        if matches!(event, UiEvent::Reconnected) && is_visible {
            load(&self.job, ctx);
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        let focus = if self.subs_area.contains(event.into()) {
            Focus::Subs
        } else if self.items_area.contains(event.into()) {
            Focus::Items
        } else {
            return Ok(());
        };
        let area = if focus == Focus::Subs { self.subs_area } else { self.items_area };
        let row = usize::from(event.y.saturating_sub(area.y + 1)); // +1: header row
        self.focus = focus;
        let (scroll, scrolloff) = (ctx.config.scroll_amount, ctx.config.scrolloff);
        match event.kind {
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                // the rows painted last: the lists change only when a render takes a new listing
                if let Some(idx) = self.state().get_at_rendered_row(row) {
                    self.state().select(Some(idx), scrolloff);
                    if focus == Focus::Subs {
                        self.sync_items(None);
                    }
                    if matches!(event.kind, MouseEventKind::DoubleClick) {
                        self.open_menu(ctx);
                    }
                }
            }
            MouseEventKind::ScrollUp => self.state().scroll_up(scroll, scrolloff),
            MouseEventKind::ScrollDown => self.state().scroll_down(scroll, scrolloff),
            _ => return Ok(()),
        }
        if focus == Focus::Subs && matches!(event.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
            self.sync_items(None);
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        let before = self.sub().map(|s| s.id.clone());
        match action {
            CommonAction::Down => self.state().next(scrolloff, wrap),
            CommonAction::Up => self.state().prev(scrolloff, wrap),
            CommonAction::DownHalf => self.state().next_half_viewport(scrolloff),
            CommonAction::UpHalf => self.state().prev_half_viewport(scrolloff),
            CommonAction::PageDown => self.state().next_viewport(scrolloff),
            CommonAction::PageUp => self.state().prev_viewport(scrolloff),
            CommonAction::Top => self.state().first(),
            CommonAction::Bottom => self.state().last(),
            CommonAction::Right if self.focus == Focus::Subs => self.focus = Focus::Items,
            CommonAction::Left if self.focus == Focus::Items => self.focus = Focus::Subs,
            // the add keys: accept the item, or (add all) every pending item of the playlist
            CommonAction::AddOptions { kind: AddKind::Action(opts) } if opts.all => {
                if let Some(sub) = self.sub() {
                    accept(ctx, &self.job, &sub.id, sub.pending());
                }
            }
            CommonAction::AddOptions { kind: AddKind::Action(_) } if self.focus == Focus::Items || self.has_marks() => {
                if let Some(sub) = self.sub() {
                    accept(ctx, &self.job, &sub.id, targets(&sub.items, &self.marked, self.item()));
                }
                self.marked.clear();
            }
            CommonAction::Delete if self.focus == Focus::Items || self.has_marks() => {
                if let Some(sub) = self.sub() {
                    reject(ctx, &self.job, &sub.id, targets(&sub.items, &self.marked, self.item()));
                }
                self.marked.clear();
            }
            // Space marks the item under the cursor (Items) and moves on
            CommonAction::Select if self.focus == Focus::Items => {
                if let Some(ytid) = self.item().map(|it| it.ytid.clone())
                    && !self.marked.remove(&ytid)
                {
                    self.marked.insert(ytid);
                }
                self.item_state.next(scrolloff, false);
            }
            CommonAction::Close if self.has_marks() => self.marked.clear(),
            CommonAction::Confirm | CommonAction::ContextMenu => {
                if self.subs.is_empty() {
                    add_url(ctx, &self.job);
                } else {
                    self.open_menu(ctx);
                }
            }
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        if self.sub().map(|s| s.id.clone()) != before {
            self.sync_items(None);
        }
        ctx.render()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `liveplaylist list --json` prints (rormpc-tools tests/test_liveplaylist.py::test_list_json_shape).
    #[test]
    fn parses_the_list_json() {
        let json = r#"{"subscriptions": [{"schema": 1, "id": "yt-PLx", "kind": "youtube", "url": "u", "title": "T",
            "playlist": "T", "dir": "LivePlaylists/T--PLx", "added_at": "2026-10-07T04:00:00",
            "last_check": {"at": "2026-10-07T04:00:00", "ok": false, "error": "HTTP Error 403", "partial": false, "new": 0},
            "counts": {"pending": 1, "accepted": 1, "rejected": 0, "queued": 0, "downloading": 0, "needs_match": 1,
                       "ready": 0, "failed": 0, "inactive": 0},
            "items": [
              {"ytid": "aaaaaaaaaa1", "title": "A", "channel": "C", "duration": 200, "decision": "pending",
               "job": null, "path": null, "first_seen": "x", "active": true, "position": 0, "last_seen": "x"},
              {"ytid": "bbbbbbbbbb2", "title": "B", "channel": "", "duration": null, "decision": "accepted",
               "job": "needs_match", "path": "/tmp/b.mp3", "source": "download", "error": null, "active": true,
               "position": 1, "review": {"reason": "no confident MusicBrainz match", "artist": "X", "title": "Y",
               "mbid": "m", "score": 0.7, "method": "lb", "alternatives": []}}
            ]}], "status": null}"#;
        let listing: Listing = serde_json::from_str(json).expect("list --json");
        let sub = &listing.subscriptions[0];
        assert_eq!(sub.pending(), vec!["aaaaaaaaaa1".to_owned()]);
        assert_eq!(sub.count("needs_match"), 1);
        assert!(sub.last_check.as_ref().is_some_and(|c| !c.ok));
        assert_eq!(job_label(&sub.items[0]), "to review");
        assert_eq!(job_label(&sub.items[1]), "needs match");
        assert_eq!(sub.items[1].review.as_ref().map(|r| r.title.as_str()), Some("Y"));
        // accept / reject act on the marked items in list order, else on the cursor item
        let marked: BTreeSet<String> = ["bbbbbbbbbb2".to_owned(), "aaaaaaaaaa1".to_owned()].into();
        assert_eq!(targets(&sub.items, &marked, None), vec!["aaaaaaaaaa1".to_owned(), "bbbbbbbbbb2".to_owned()]);
        assert_eq!(targets(&sub.items, &BTreeSet::new(), sub.items.get(1)), vec!["bbbbbbbbbb2".to_owned()]);
        assert!(targets(&sub.items, &BTreeSet::new(), None).is_empty());
    }

    #[test]
    fn parses_the_status_file() {
        let json = r#"{"pid": 1, "updated_at": "x", "running": true, "state": "running", "subscription": "yt-PLx",
            "current": {"ytid": "aaaaaaaaaa1", "title": "A"}, "done": 1, "failed": 0, "total": 3, "errors": []}"#;
        let st: Status = serde_json::from_str(json).expect("status.json");
        assert!(st.running && st.total == 3 && st.current.is_some_and(|c| c.title == "A"));
    }
}
