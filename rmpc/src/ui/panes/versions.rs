//! rormpc: Versions pane. Song names that several different library files share, from `musicdb versions --json`
//! (rormpc-tools): which file each played track (a Spotify URI, a recording MBID, or the name) is, or "a version I
//! don't own"; version labels; merging files that are one recording; and YouTube ids / MBIDs on several files to
//! review. Every decision is a `musicdb versions ...` call; the list reloads after each. Previews play in a
//! separate player (mpv, else ffplay), never through MPD, whose scrobbler would log a listen or a skip.

use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
};
use rmpc_mpd::commands::State;
use serde::Deserialize;

use super::Pane;
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        macros::{modal, status_error, status_info, status_warn},
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        UiEvent,
        dirstack::DirState,
        input::{BufferId, InputResultEvent},
        modals::{
            confirm_modal::{Action, ConfirmModal},
            menu::modal::MenuModal,
        },
        rormpc_filter::{Query, binding},
    },
};

const MUSICDB: &str = "musicdb";
const LABELS: [&str; 6] = ["original", "live", "remix", "edit", "cover", "other"];
/// where a preview starts, in seconds (long video intros make 0:00 useless)
const STARTS: [u32; 4] = [0, 30, 60, 120];

#[derive(Debug, Clone, Default, Deserialize)]
struct Report {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    music_dir: Option<String>,
    #[serde(default)]
    groups: Vec<Group>,
    #[serde(default)]
    shared: Vec<Shared>,
}

#[derive(Debug, Clone, Deserialize)]
struct Group {
    name: String,
    #[serde(default)]
    files: Vec<VFile>,
    #[serde(default)]
    tracks: Vec<Track>,
    #[serde(default)]
    pending: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct VFile {
    file: String,
    #[serde(default)]
    duration_s: u64,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    markers: Vec<String>,
    #[serde(default)]
    plays: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct Track {
    source: String,
    track: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    plays: u32,
    #[serde(default)]
    first: String,
    #[serde(default)]
    last: String,
    #[serde(default)]
    longest_s: u64,
    #[serde(default)]
    decision: Option<Decision>,
    #[serde(default)]
    suggest: Option<Suggest>,
}

#[derive(Debug, Clone, Deserialize)]
struct Decision {
    action: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    stale: bool,
    #[serde(default)]
    group_changed: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Suggest {
    file: String,
    reason: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Shared {
    id: String,
    #[serde(default)]
    files: Vec<SharedFile>,
}

#[derive(Debug, Clone, Deserialize)]
struct SharedFile {
    file: String,
    #[serde(default)]
    duration_s: u64,
}

/// A row of the left list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Group(usize),
    Shared(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Files,
    Tracks,
}

/// State shared with the background `musicdb` runs.
#[derive(Debug, Default)]
struct Job {
    loading: bool,
    report: Option<Report>,
    error: Option<String>,
    reload: bool,
}

/// The preview player: its process, file and start.
#[derive(Debug, Default)]
struct Preview {
    child: Option<Child>,
    what: String,
}

impl Preview {
    fn stop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        self.what.clear();
    }

    /// Still playing? Clears itself when the player has ended.
    fn playing(&mut self) -> bool {
        match self.child.as_mut().map(Child::try_wait) {
            Some(Ok(None)) => true,
            Some(_) => {
                self.child = None;
                self.what.clear();
                false
            }
            None => false,
        }
    }
}

#[derive(Debug)]
pub struct VersionsPane {
    report: Report,
    entries: Vec<Entry>,
    state: DirState<TableState>,
    list_area: Rect,
    focus: Focus,
    file_idx: usize,
    track_idx: usize,
    job: Arc<Mutex<Job>>,
    preview: Arc<Mutex<Preview>>,
    /// the live filter of the group list: its text, whether it takes keys now, "unresolved only"
    filter: BufferId,
    typing: bool,
    query: String,
    unresolved_only: bool,
    /// the selected entry before filtering began, restored by Esc
    before_filter: Option<String>,
}

fn run(args: &[&str]) -> Result<String, String> {
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

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default()
}

fn expand(p: &str) -> PathBuf {
    p.strip_prefix("~/").map_or_else(|| PathBuf::from(p), |rest| home().join(rest))
}

/// The music directory: from musicdb's JSON, else rormpc-tools' setting ($YTMB_MUSIC_DIR, `music_dir` in its
/// config.toml), else ~/Music, its default.
fn music_dir(report: &Report) -> PathBuf {
    if let Some(d) = report.music_dir.as_deref().filter(|d| !d.is_empty()) {
        return expand(d);
    }
    if let Some(d) = std::env::var_os("YTMB_MUSIC_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let config = std::env::var_os("RORMPC_TOOLS_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config/rormpc-tools/config.toml"));
    std::fs::read_to_string(config)
        .ok()
        .and_then(|text| config_music_dir(&text))
        .map_or_else(|| home().join("Music"), |d| expand(&d))
}

/// `music_dir = "..."` from a TOML file, without a TOML parser (one plain string key).
fn config_music_dir(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "music_dir").then(|| v.trim().trim_matches('"').to_owned())
    })
}

/// The player command for a preview: mpv, else ffplay; none if neither is installed.
fn player_command(path: &std::path::Path, start: u32) -> Option<Command> {
    if which::which("mpv").is_ok() {
        let mut c = Command::new("mpv");
        c.args(["--no-video", "--really-quiet", &format!("--start={start}")]).arg(path);
        return Some(c);
    }
    if which::which("ffplay").is_ok() {
        let mut c = Command::new("ffplay");
        c.args(["-nodisp", "-autoexit", "-loglevel", "quiet", "-ss", &start.to_string()]).arg(path);
        return Some(c);
    }
    None
}

fn mmss(s: u64) -> String {
    format!("{}:{:02}", s / 60, s % 60)
}

/// musicdb's suggestion reason for people: "longest play 207 s, file 218 s; 443 s does not fit" ->
/// "longest listen 3:27, file 3:38; 7:23 does not fit".
fn human_reason(reason: &str) -> String {
    let reason = reason.replace("longest play", "longest listen");
    let mut out = String::new();
    let mut rest = reason.as_str();
    while let Some(pos) = rest.find(|c: char| c.is_ascii_digit()) {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        let digits = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
        let (num, after) = rest.split_at(digits);
        let is_seconds = after.strip_prefix(" s").is_some_and(|a| a.chars().next().is_none_or(|c| !c.is_alphanumeric()));
        match (is_seconds, num.parse::<u64>()) {
            (true, Ok(n)) => {
                out.push_str(&mmss(n));
                rest = &after[2..];
            }
            _ => {
                out.push_str(num);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn base(f: &str) -> &str {
    f.rsplit('/').next().unwrap_or(f)
}

fn group_plays(g: &Group) -> u32 {
    g.tracks.iter().map(|t| t.plays).sum()
}

/// Open groups first, most plays first; then the shared ids. Shared ids are always unresolved.
fn entries_filtered(report: &Report, query: &str, unresolved_only: bool) -> Vec<Entry> {
    let mut fuzzy = Query::new(query);
    entries(report)
        .into_iter()
        .filter(|e| match *e {
            Entry::Group(i) => {
                let g = &report.groups[i];
                (!unresolved_only || !g.pending.is_empty()) && fuzzy.matches(&g.name.replace('|', " - "))
            }
            Entry::Shared(i) => fuzzy.matches(&report.shared[i].id),
        })
        .collect()
}

/// Open groups first, most plays first; then the shared ids.
fn entries(report: &Report) -> Vec<Entry> {
    let mut groups: Vec<usize> = (0..report.groups.len()).collect();
    groups.sort_by_key(|&i| {
        let g = &report.groups[i];
        (g.pending.is_empty(), std::cmp::Reverse(group_plays(g)), g.name.clone())
    });
    groups.into_iter().map(Entry::Group).chain((0..report.shared.len()).map(Entry::Shared)).collect()
}

fn open_items(report: &Report) -> usize {
    report.groups.iter().map(|g| g.pending.len()).sum::<usize>() + report.shared.len()
}

fn decision_text(t: &Track, files: &[VFile]) -> String {
    match &t.decision {
        None => "Unresolved".to_owned(),
        Some(d) if d.stale => "review: file gone".to_owned(),
        Some(d) if d.group_changed => "review: group changed".to_owned(),
        Some(d) if d.action == "none" => "not owned".to_owned(),
        Some(d) => d
            .file
            .as_deref()
            .and_then(|f| files.iter().position(|x| x.file == f))
            .map_or_else(|| "-> ?".to_owned(), |i| format!("-> {}", i + 1)),
    }
}

/// Arguments of the `musicdb` call for each decision (kept here so tests check them).
fn args_set(t: &Track, file: &str) -> Vec<String> {
    vec!["versions".into(), "set".into(), t.source.clone(), t.track.clone(), file.to_owned()]
}

fn args_none(t: &Track) -> Vec<String> {
    vec!["versions".into(), "none".into(), t.source.clone(), t.track.clone()]
}

fn args_clear(t: &Track) -> Vec<String> {
    vec!["versions".into(), "clear".into(), t.source.clone(), t.track.clone()]
}

fn args_label(file: &str, version: &str) -> Vec<String> {
    vec!["versions".into(), "label".into(), file.to_owned(), version.to_owned()]
}

fn args_same(keep: &str, others: &[String]) -> Vec<String> {
    let mut a = vec!["versions".into(), "same".into(), keep.to_owned()];
    a.extend(others.iter().cloned());
    a
}

fn args_shared_ok(id: &str) -> Vec<String> {
    vec!["versions".into(), "shared-ok".into(), id.to_owned()]
}

/// Run `musicdb` for a decision in the background, report its last line, then reload the list.
fn run_then_reload(ctx: &Ctx, job: Arc<Mutex<Job>>, args: Vec<String>) {
    let sender = ctx.app_event_sender.clone();
    std::thread::spawn(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match run(&args) {
            Ok(out) => {
                let last = out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("done").to_owned();
                status_info!("{last}");
            }
            Err(err) => status_error!("musicdb: {err}"),
        }
        job.lock().expect("versions job lock").reload = true;
        let _ = sender.send(AppEvent::RequestRender);
    });
}

/// At startup: warn once when the hourly `musicdb update` found open items (~/.cache/rormpc-tools/doctor.json).
fn warn_from_doctor() {
    #[derive(Deserialize)]
    struct Doctor {
        version: u32,
        clean: bool,
        #[serde(default)]
        counts: std::collections::BTreeMap<String, u64>,
    }
    let cache = std::env::var_os("XDG_CACHE_HOME").map_or_else(|| home().join(".cache"), PathBuf::from);
    let Some(d) = std::fs::read_to_string(cache.join("rormpc-tools/doctor.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Doctor>(&t).ok())
    else {
        return;
    };
    if d.version == 1 && !d.clean {
        let n: u64 = d.counts.values().sum();
        status_warn!("doctor: {n} open (Versions pane)");
    }
}

impl VersionsPane {
    pub fn new() -> Self {
        std::thread::spawn(warn_from_doctor);
        Self {
            report: Report::default(),
            entries: Vec::new(),
            state: DirState::default(),
            list_area: Rect::default(),
            focus: Focus::List,
            file_idx: 0,
            track_idx: 0,
            job: Arc::new(Mutex::new(Job::default())),
            preview: Arc::new(Mutex::new(Preview::default())),
            filter: BufferId::new(),
            typing: false,
            query: String::new(),
            unresolved_only: false,
            before_filter: None,
        }
    }

    fn entry_key(&self, e: Entry) -> Option<String> {
        match e {
            Entry::Group(i) => self.report.groups.get(i).map(|g| format!("g:{}", g.name)),
            Entry::Shared(i) => self.report.shared.get(i).map(|s| format!("s:{}", s.id)),
        }
    }

    fn filter_active(&self) -> bool {
        self.typing || !self.query.trim().is_empty() || self.unresolved_only
    }

    /// Recompute the visible entries; keep `keep` (an entry key) selected when it is still visible, else the
    /// first entry. Files and tracks inside a group are never filtered.
    fn refilter(&mut self, keep: Option<String>) {
        self.entries = entries_filtered(&self.report, &self.query, self.unresolved_only);
        self.state.set_content_and_viewport_len(self.entries.len(), self.list_area.height.into());
        let idx = keep
            .and_then(|k| self.entries.iter().position(|&e| self.entry_key(e).as_deref() == Some(k.as_str())))
            .unwrap_or(0);
        self.state.select((!self.entries.is_empty()).then_some(idx), 0);
        self.file_idx = 0;
        self.track_idx = 0;
        self.clamp();
    }

    fn start_filter(&mut self, ctx: &Ctx) {
        if !self.filter_active() {
            self.before_filter = self.entry().and_then(|e| self.entry_key(e));
        }
        self.focus = Focus::List;
        self.typing = true;
        ctx.input.insert_mode(self.filter);
    }

    /// Esc: no filter, the selection from before it.
    fn clear_filter(&mut self, ctx: &Ctx) {
        ctx.input.clear_buffer(self.filter);
        self.query.clear();
        self.typing = false;
        self.unresolved_only = false;
        let keep = self.before_filter.take();
        self.refilter(keep);
    }

    /// The hint line: the keys that work where the cursor is, from the real bindings.
    fn hints(&self, ctx: &Ctx) -> String {
        let nav = &ctx.config.keybinds.navigation;
        let key = |want: fn(&CommonAction) -> bool, fallback: &str| binding(nav, want).unwrap_or_else(|| fallback.to_owned());
        let enter = key(|a| matches!(a, CommonAction::Confirm), "Enter");
        let close = key(|a| matches!(a, CommonAction::Close), "Esc");
        if self.typing {
            return format!("type to filter groups · ↑/↓ move · {enter} keep the filter · {close} clear it");
        }
        let left = key(|a| matches!(a, CommonAction::Left), "h");
        let right = key(|a| matches!(a, CommonAction::Right), "l");
        let search = key(|a| matches!(a, CommonAction::EnterSearch), "/");
        let select = key(|a| matches!(a, CommonAction::Select), "Space");
        let mut h = match self.focus {
            Focus::List => format!(
                "{enter} open the group · {right} files · {search} filter · {select} unresolved only ({})",
                if self.unresolved_only { "on" } else { "off" }
            ),
            Focus::Files => {
                format!("{enter} actions: preview, label, same recording · {left}/{right} switch list · {search} filter")
            }
            Focus::Tracks => format!(
                "{enter} actions: accept the suggestion, which file it is, not owned · {left} files · {search} filter"
            ),
        };
        if self.filter_active() && self.focus == Focus::List {
            h.push_str(&format!(" · {close} clear the filter"));
        }
        h
    }

    fn load(&self, ctx: &Ctx) {
        let mut job = self.job.lock().expect("versions job lock");
        if job.loading {
            return;
        }
        job.loading = true;
        drop(job);
        let (job, sender) = (Arc::clone(&self.job), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let result = run(&["versions", "--json"])
                .and_then(|out| serde_json::from_str::<Report>(&out).map_err(|e| e.to_string()))
                .and_then(|r| {
                    (r.version == 1).then_some(r).ok_or_else(|| "unknown versions JSON (update rormpc)".to_owned())
                });
            let mut j = job.lock().expect("versions job lock");
            j.loading = false;
            match result {
                Ok(r) => {
                    j.report = Some(r);
                    j.error = None;
                }
                Err(err) => j.error = Some(err),
            }
            drop(j);
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    fn entry(&self) -> Option<Entry> {
        self.state.get_selected().and_then(|i| self.entries.get(i).copied())
    }

    fn group(&self) -> Option<&Group> {
        match self.entry()? {
            Entry::Group(i) => self.report.groups.get(i),
            Entry::Shared(_) => None,
        }
    }

    fn shared(&self) -> Option<&Shared> {
        match self.entry()? {
            Entry::Shared(i) => self.report.shared.get(i),
            Entry::Group(_) => None,
        }
    }

    /// Files of the selected entry: (path, length).
    fn files(&self) -> Vec<(String, u64)> {
        if let Some(g) = self.group() {
            return g.files.iter().map(|f| (f.file.clone(), f.duration_s)).collect();
        }
        self.shared().map_or_else(Vec::new, |s| s.files.iter().map(|f| (f.file.clone(), f.duration_s)).collect())
    }

    fn tracks_len(&self) -> usize {
        self.group().map_or(0, |g| g.tracks.len())
    }

    /// Take a new report, keeping the selected entry (by name / id) and the positions inside it.
    fn take_report(&mut self, report: Report) {
        let keep = self.entry().map(|e| self.entry_key(e));
        let (file_idx, track_idx) = (self.file_idx, self.track_idx);
        self.report = report;
        let keep = keep.flatten();
        let same = keep.is_some();
        self.refilter(keep);
        if same {
            self.file_idx = file_idx;
            self.track_idx = track_idx;
            self.clamp();
        }
    }

    fn clamp(&mut self) {
        self.file_idx = self.file_idx.min(self.files().len().saturating_sub(1));
        self.track_idx = self.track_idx.min(self.tracks_len().saturating_sub(1));
        if self.focus == Focus::Tracks && self.tracks_len() == 0 {
            self.focus = Focus::Files;
        }
    }

    fn stop_preview(&self) {
        self.preview.lock().expect("preview lock").stop();
    }

    /// Enter / context menu on the focused list.
    fn open_menu(&mut self, ctx: &Ctx) {
        match self.focus {
            Focus::List => {
                self.focus = if self.tracks_len() > 0 { Focus::Tracks } else { Focus::Files };
            }
            Focus::Tracks => self.track_menu(ctx),
            Focus::Files => self.file_menu(ctx),
        }
    }

    fn track_menu(&self, ctx: &Ctx) {
        let Some(g) = self.group().cloned() else { return };
        let Some(t) = g.tracks.get(self.track_idx).cloned() else { return };
        let job = Arc::clone(&self.job);
        let menu = MenuModal::new(ctx)
            .width(80) // file names are long
            .list_section(ctx, move |mut section| {
                if let (Some(s), None) = (&t.suggest, &t.decision) {
                    let (job, args) = (Arc::clone(&job), args_set(&t, &s.file));
                    section.add_item(format!("Accept suggestion: {}", base(&s.file)), move |ctx| {
                        run_then_reload(ctx, job, args);
                        Ok(())
                    });
                }
                for (i, f) in g.files.iter().enumerate() {
                    let (job, args) = (Arc::clone(&job), args_set(&t, &f.file));
                    let label = f.version.as_deref().unwrap_or("?");
                    section.add_item(
                        format!("{}. It is {} ({}, {label})", i + 1, base(&f.file), mmss(f.duration_s)),
                        move |ctx| {
                            run_then_reload(ctx, job, args);
                            Ok(())
                        },
                    );
                }
                let (j2, args) = (Arc::clone(&job), args_none(&t));
                section.add_item("A version I don't own", move |ctx| {
                    run_then_reload(ctx, j2, args);
                    Ok(())
                });
                if t.decision.is_some() {
                    let args = args_clear(&t);
                    section.add_item("Clear the decision", move |ctx| {
                        run_then_reload(ctx, job, args);
                        Ok(())
                    });
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    fn file_menu(&self, ctx: &Ctx) {
        let files = self.files();
        let Some((file, _)) = files.get(self.file_idx).cloned() else { return };
        let group = self.group().cloned();
        let shared = self.shared().cloned();
        let playing = self.preview.lock().expect("preview lock").playing();
        let (job, preview, dir) = (Arc::clone(&self.job), Arc::clone(&self.preview), music_dir(&self.report));
        let others: Vec<String> = files.iter().map(|(f, _)| f.clone()).filter(|f| *f != file).collect();
        let menu = MenuModal::new(ctx)
            .width(80) // file names are long
            .list_section(ctx, {
                let (file, preview) = (file.clone(), Arc::clone(&preview));
                move |mut section| {
                    for start in STARTS {
                        let (file, preview, dir) = (file.clone(), Arc::clone(&preview), dir.clone());
                        section.add_item(format!("Preview from {}", mmss(start.into())), move |_| {
                            start_preview(&preview, &dir, &file, start);
                            Ok(())
                        });
                    }
                    if playing {
                        section.add_item("Stop the preview", move |_| {
                            preview.lock().expect("preview lock").stop();
                            Ok(())
                        });
                    }
                    Some(section)
                }
            })
            .list_section(ctx, {
                let (file, job) = (file.clone(), Arc::clone(&job));
                move |mut section| {
                    let current = group
                        .as_ref()
                        .and_then(|g| g.files.iter().find(|f| f.file == file))
                        .and_then(|f| f.version.clone());
                    if group.is_some() {
                        for v in LABELS.iter().copied().chain((current.is_some()).then_some("clear")) {
                            let (job, args) = (Arc::clone(&job), args_label(&file, v));
                            let mark = if current.as_deref() == Some(v) { " ✓" } else { "" };
                            section.add_item(format!("Label: {v}{mark}"), move |ctx| {
                                run_then_reload(ctx, job, args);
                                Ok(())
                            });
                        }
                    }
                    if !others.is_empty() && group.is_some() {
                        let (job, file, others) = (Arc::clone(&job), file.clone(), others.clone());
                        section.add_item("Same recording: keep this file, merge the others…", move |ctx| {
                            confirm_same(ctx, job, file, others);
                            Ok(())
                        });
                    }
                    if let Some(s) = shared {
                        section.add_item("These files are fine (shared-ok)…", move |ctx| {
                            confirm_shared_ok(ctx, job, s);
                            Ok(())
                        });
                    }
                    Some(section)
                }
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    fn move_in(&mut self, down: bool, ctx: &Ctx) {
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        let step = |i: usize, len: usize| -> usize {
            if len == 0 {
                0
            } else if down {
                if i + 1 < len { i + 1 } else if wrap { 0 } else { i }
            } else if i > 0 {
                i - 1
            } else if wrap {
                len - 1
            } else {
                0
            }
        };
        match self.focus {
            Focus::List => {
                if down {
                    self.state.next(scrolloff, wrap);
                } else {
                    self.state.prev(scrolloff, wrap);
                }
                self.file_idx = 0;
                self.track_idx = 0;
                self.clamp();
            }
            Focus::Files => self.file_idx = step(self.file_idx, self.files().len()),
            Focus::Tracks => self.track_idx = step(self.track_idx, self.tracks_len()),
        }
    }

    fn details(&self) -> (Vec<Line<'static>>, Vec<Row<'static>>, Vec<Row<'static>>) {
        let dim = Style::default().add_modifier(Modifier::DIM);
        let mut head = Vec::new();
        let mut files = Vec::new();
        let mut tracks = Vec::new();
        if let Some(g) = self.group() {
            head.push(Line::from(Span::styled(g.name.clone(), Style::default().add_modifier(Modifier::BOLD))));
            head.push(Line::from(Span::styled(
                if g.pending.is_empty() { "nothing open".to_owned() } else { format!("{} open", g.pending.len()) },
                dim,
            )));
            for (i, f) in g.files.iter().enumerate() {
                let marks = if f.markers.is_empty() { String::new() } else { format!(" [{}]", f.markers.join(",")) };
                files.push(Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(mmss(f.duration_s)),
                    Cell::from(f.version.clone().unwrap_or_else(|| "?".to_owned())),
                    Cell::from(f.plays.to_string()),
                    Cell::from(format!("{}{marks}", base(&f.file))),
                ]));
            }
            for t in &g.tracks {
                let open = t.decision.as_ref().is_none_or(|d| d.stale || d.group_changed);
                let sug = match (&t.suggest, &t.decision) {
                    (Some(s), None) => {
                        g.files.iter().position(|f| f.file == s.file).map_or_else(String::new, |i| format!("file {}", i + 1))
                    }
                    _ => String::new(),
                };
                tracks.push(
                    Row::new(vec![
                        Cell::from(t.source.clone()),
                        Cell::from(t.plays.to_string()),
                        Cell::from(t.album.clone().unwrap_or_default()),
                        Cell::from(format!(
                            "{}–{}",
                            t.first.get(..10).unwrap_or(&t.first),
                            t.last.get(..10).unwrap_or(&t.last)
                        )),
                        Cell::from(if t.longest_s > 0 { mmss(t.longest_s) } else { String::new() }),
                        Cell::from(decision_text(t, &g.files)),
                        Cell::from(sug),
                    ])
                    .style(if open { Style::default().add_modifier(Modifier::BOLD) } else { Style::default() }),
                );
            }
        } else if let Some(s) = self.shared() {
            head.push(Line::from(Span::styled(s.id.clone(), Style::default().add_modifier(Modifier::BOLD))));
            head.push(Line::from(Span::styled(
                "one id on several files: identical audio -> musicdb dedupe, one recording -> Same recording in its \
                 group, right as it is -> shared-ok, a wrong tag -> fix the tag",
                dim,
            )));
            for (i, f) in s.files.iter().enumerate() {
                files.push(Row::new(vec![
                    Cell::from(format!("{}", i + 1)),
                    Cell::from(mmss(f.duration_s)),
                    Cell::from(""),
                    Cell::from(""),
                    Cell::from(base(&f.file).to_owned()),
                ]));
            }
        }
        (head, files, tracks)
    }

    fn selected_track_lines(&self, ctx: &Ctx) -> Vec<Line<'static>> {
        let key = ctx.config.theme.preview_label_style;
        let Some(g) = self.group() else { return Vec::new() };
        let Some(t) = g.tracks.get(self.track_idx) else { return Vec::new() };
        let field = |name: &str, value: String| Line::from(vec![Span::styled(format!("{name}: "), key), Span::raw(value)]);
        let mut lines = vec![field("Track", format!("{} - {}  ({})", t.artist, t.title, t.track))];
        if let Some(s) = t.suggest.as_ref().filter(|_| t.decision.is_none()) {
            lines.push(field("Suggested", base(&s.file).to_owned()));
            lines.push(field("Why", human_reason(&s.reason)));
        }
        if t.longest_s > 0 {
            lines.push(Line::from(Span::styled(
                "Spotify records how long you listened each time; the longest listen is about the song's length if \
                 you ever played it to the end.",
                Style::default().add_modifier(Modifier::DIM),
            )));
        }
        lines
    }
}

/// Play `dir/file` from `start` in the preview player, replacing a running preview.
fn start_preview(preview: &Mutex<Preview>, dir: &std::path::Path, file: &str, start: u32) {
    let mut p = preview.lock().expect("preview lock");
    p.stop();
    let Some(mut cmd) = player_command(&dir.join(file), start) else {
        status_error!("preview needs mpv or ffplay");
        return;
    };
    match cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() {
        Ok(child) => {
            p.child = Some(child);
            p.what = format!("{} from {}", base(file), mmss(start.into()));
        }
        Err(err) => status_error!("preview: {err}"),
    }
}

/// "Same recording": the confirmation names every file merged into the kept one; Cancel is first.
fn confirm_same(ctx: &Ctx, job: Arc<Mutex<Job>>, keep: String, others: Vec<String>) {
    let message = format!(
        "Keep {} and merge into it (like, missing tags, lyrics; the others go to the quarantine): {}",
        base(&keep),
        others.iter().map(|f| base(f)).collect::<Vec<_>>().join(", ")
    );
    let args = args_same(&keep, &others);
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(vec![message])
            .action(Action::CustomButtons {
                buttons: vec![
                    ("Cancel", Box::new(|_: &Ctx| Ok(()))),
                    (
                        "Merge",
                        Box::new(move |ctx: &Ctx| {
                            run_then_reload(ctx, job, args);
                            Ok(())
                        }),
                    ),
                ],
            })
            .build()
    );
}

fn confirm_shared_ok(ctx: &Ctx, job: Arc<Mutex<Job>>, s: Shared) {
    let message = format!(
        "{} on {} is right as it is (stops reporting it): {}",
        s.id,
        if s.files.len() == 2 { "both files".to_owned() } else { format!("{} files", s.files.len()) },
        s.files.iter().map(|f| base(&f.file)).collect::<Vec<_>>().join(", ")
    );
    let args = args_shared_ok(&s.id);
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(vec![message])
            .action(Action::CustomButtons {
                buttons: vec![
                    ("Cancel", Box::new(|_: &Ctx| Ok(()))),
                    (
                        "Mark as fine",
                        Box::new(move |ctx: &Ctx| {
                            run_then_reload(ctx, job, args);
                            Ok(())
                        }),
                    ),
                ],
            })
            .build()
    );
}

impl Drop for VersionsPane {
    fn drop(&mut self) {
        self.stop_preview(); // never leave a player running after rormpc quits
    }
}

impl Pane for VersionsPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let (fresh, reload, loading, error) = {
            let mut j = self.job.lock().expect("versions job lock");
            (j.report.take(), std::mem::take(&mut j.reload), j.loading, j.error.clone())
        };
        if reload {
            self.load(ctx);
        }
        if let Some(r) = fresh {
            self.take_report(r);
        }
        let [body, hint_area] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let [left, right] = Layout::horizontal([Constraint::Percentage(30), Constraint::Min(40)]).spacing(2).areas(body);
        let filter_rows = u16::from(self.filter_active());
        let [filter_area, list_area, footer] =
            Layout::vertical([Constraint::Length(filter_rows), Constraint::Min(1), Constraint::Length(1)]).areas(left);
        self.list_area = list_area;
        self.state.set_content_and_viewport_len(self.entries.len(), list_area.height.saturating_sub(1).into());

        let dim = Style::default().add_modifier(Modifier::DIM);
        let focused = |f: Focus| if self.focus == f { ctx.config.theme.current_item_style } else { Style::default() };
        let rows = self.entries.iter().map(|e| match *e {
            Entry::Group(i) => {
                let g = &self.report.groups[i];
                Row::new(vec![
                    Cell::from(if g.pending.is_empty() { "✓".to_owned() } else { g.pending.len().to_string() }),
                    Cell::from(group_plays(g).to_string()),
                    Cell::from(g.name.replace('|', " - ")),
                ])
                .style(if g.pending.is_empty() { dim } else { Style::default() })
            }
            Entry::Shared(i) => Row::new(vec![
                Cell::from("id"),
                Cell::from(""),
                Cell::from(format!("shared {}", self.report.shared[i].id)),
            ]),
        });
        let header = Row::new(["Open", "Plays", "Name"]).style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [Constraint::Length(4), Constraint::Length(5), Constraint::Min(10)])
            .header(header)
            .column_spacing(1)
            .style(ctx.config.as_text_style())
            .row_highlight_style(focused(Focus::List).add_modifier(Modifier::REVERSED));
        frame.render_stateful_widget(table, list_area, self.state.as_render_state_ref());

        if filter_rows > 0 {
            let total = self.report.groups.len() + self.report.shared.len();
            let mut text = format!("Filter: {}{}", self.query, if self.typing { "▏" } else { "" });
            if self.unresolved_only {
                text.push_str(" · unresolved only");
            }
            let count = format!(" {}/{total}", self.entries.len());
            let [text_area, count_area] =
                Layout::horizontal([Constraint::Min(1), Constraint::Length(count.chars().count() as u16)]).areas(filter_area);
            let style = if self.typing { ctx.config.theme.highlight_border_style } else { ctx.config.as_text_style() };
            frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), text_area);
            frame.render_widget(Paragraph::new(Line::from(count)).style(ctx.config.as_text_style()), count_area);
        }

        let open = open_items(&self.report);
        let mut preview = self.preview.lock().expect("preview lock");
        let preview_text = if preview.playing() { format!(" · preview: {}", preview.what) } else { String::new() };
        drop(preview);
        let mpd_note =
            if !preview_text.is_empty() && matches!(ctx.status.state, State::Play) { " (MPD is playing too)" } else { "" };
        let status = match (loading, error) {
            (_, Some(err)) => Span::styled(format!(" musicdb: {err}"), Style::default().add_modifier(Modifier::BOLD)),
            (true, None) if self.entries.is_empty() => Span::styled(" reading musicdb versions…", dim),
            _ if open == 0 => Span::styled(format!(" clean{preview_text}{mpd_note}"), dim),
            _ => Span::styled(format!(" open items: {open}{preview_text}{mpd_note}"), dim),
        };
        frame.render_widget(Paragraph::new(Line::from(status)), footer);

        let (head, files, tracks) = self.details();
        let [head_area, files_area, tracks_area, info_area] = Layout::vertical([
            Constraint::Length(head.len().max(1) as u16 + 1),
            Constraint::Length(files.len() as u16 + 2),
            Constraint::Min(3),
            Constraint::Length(9), // track, suggested file and why (both wrap), the listen-length note
        ])
        .areas(right);
        frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), head_area);

        let label = ctx.config.theme.preview_label_style;
        let files_table = Table::new(files, [
            Constraint::Length(2),
            Constraint::Length(6),
            Constraint::Length(9),
            Constraint::Length(5),
            Constraint::Min(10),
        ])
        .header(Row::new(["#", "Length", "Version", "Plays", "File"]).style(label))
        .column_spacing(1)
        .row_highlight_style(focused(Focus::Files).add_modifier(Modifier::REVERSED));
        let mut fs = TableState::default().with_selected((self.focus == Focus::Files).then_some(self.file_idx));
        frame.render_stateful_widget(files_table, files_area, &mut fs);

        if self.group().is_some() {
            let tracks_table = Table::new(tracks, [
                Constraint::Length(7),
                Constraint::Length(5),
                Constraint::Percentage(25),
                Constraint::Length(21),
                Constraint::Length(7),
                Constraint::Length(21),
                Constraint::Length(9),
            ])
            .header(Row::new(["Source", "Plays", "Album", "Played", "Longest", "Decision", "Suggested"]).style(label))
            .column_spacing(1)
            .row_highlight_style(focused(Focus::Tracks).add_modifier(Modifier::REVERSED));
            let mut ts = TableState::default().with_selected((self.focus == Focus::Tracks).then_some(self.track_idx));
            frame.render_stateful_widget(tracks_table, tracks_area, &mut ts);
        }
        let info = if self.focus == Focus::Tracks { self.selected_track_lines(ctx) } else { Vec::new() };
        frame.render_widget(Paragraph::new(info).wrap(Wrap { trim: false }), info_area);
        frame.render_widget(Paragraph::new(Line::from(Span::styled(self.hints(ctx), dim))), hint_area);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.load(ctx);
        Ok(())
    }

    fn on_hide(&mut self, _ctx: &Ctx) -> Result<()> {
        self.stop_preview();
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        // a download or a deletion changes the groups
        if matches!(event, UiEvent::Database | UiEvent::Reconnected) && is_visible {
            self.load(ctx);
        }
        Ok(())
    }

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &mut Ctx) -> Result<()> {
        match kind {
            InputResultEvent::Push | InputResultEvent::Pop => {
                self.query = ctx.input.value(self.filter);
                let keep = self.entry().and_then(|e| self.entry_key(e));
                self.refilter(keep);
            }
            InputResultEvent::Confirm => self.typing = false, // Enter keeps the filter
            InputResultEvent::Cancel => self.clear_filter(ctx),
            InputResultEvent::NoChange => {}
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_insert_nav(&mut self, down: bool, handled: &mut bool, ctx: &mut Ctx) -> Result<()> {
        if self.typing {
            self.move_in(down, ctx);
            *handled = true;
            ctx.render()?;
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if !self.list_area.contains(event.into()) {
            return Ok(());
        }
        let row = usize::from(event.y.saturating_sub(self.list_area.y + 1)); // +1: header row
        match event.kind {
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                if let Some(idx) = self.state.get_at_rendered_row(row) {
                    self.state.select(Some(idx), ctx.config.scrolloff);
                    self.focus = Focus::List;
                    self.file_idx = 0;
                    self.track_idx = 0;
                    self.clamp();
                    if matches!(event.kind, MouseEventKind::DoubleClick) {
                        self.open_menu(ctx);
                    }
                }
            }
            MouseEventKind::ScrollUp => self.state.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff),
            MouseEventKind::ScrollDown => self.state.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff),
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        match action {
            CommonAction::Down => self.move_in(true, ctx),
            CommonAction::Up => self.move_in(false, ctx),
            CommonAction::Right => {
                self.focus = match self.focus {
                    Focus::List => Focus::Files,
                    Focus::Files if self.tracks_len() > 0 => Focus::Tracks,
                    f => f,
                };
                self.clamp();
            }
            CommonAction::Left => {
                self.focus = match self.focus {
                    Focus::Tracks => Focus::Files,
                    _ => Focus::List,
                };
            }
            CommonAction::Top if self.focus == Focus::List => self.state.first(),
            CommonAction::Bottom if self.focus == Focus::List => self.state.last(),
            CommonAction::Confirm | CommonAction::ContextMenu => self.open_menu(ctx),
            CommonAction::Close if self.focus != Focus::List => self.focus = Focus::List,
            CommonAction::EnterSearch | CommonAction::FocusInput => self.start_filter(ctx),
            CommonAction::Select if self.focus == Focus::List => {
                self.unresolved_only = !self.unresolved_only;
                if self.unresolved_only && self.before_filter.is_none() && self.query.trim().is_empty() {
                    self.before_filter = self.entry().and_then(|e| self.entry_key(e));
                }
                let keep = self.entry().and_then(|e| self.entry_key(e));
                self.refilter(keep);
            }
            CommonAction::Close if self.filter_active() => self.clear_filter(ctx),
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        ctx.render()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{"version": 1, "music_dir": "/m", "groups": [
      {"name": "tiesto|adagio for strings", "pending": ["track spotify spotify:track:x", "label a.mp3"],
       "files": [{"key": "file:a.mp3", "file": "Mix/a.mp3", "artist": "T", "title": "A", "duration_s": 218,
                  "mbid": null, "version": null, "markers": ["live"], "plays": 0},
                 {"key": "yt:abc", "file": "Mix/b.mp3", "duration_s": 443, "version": "original", "plays": 3}],
       "tracks": [{"source": "spotify", "track": "spotify:track:x", "artist": "Tiësto", "title": "Adagio",
                   "album": "ASOT", "plays": 26, "first": "2015-01-01T10:00:00", "last": "2016-01-01T10:00:00",
                   "longest_s": 440, "decision": null, "suggest": {"file": "Mix/b.mp3", "reason": "longest play"}}]},
      {"name": "done|song", "pending": [], "files": [], "tracks": []}],
     "shared": [{"id": "mb:1", "files": [{"file": "x.mp3", "duration_s": 200}, {"file": "y.mp3", "duration_s": 300}]}]}"#;

    #[test]
    fn parses_the_report_and_orders_open_groups_first() {
        let r: Report = serde_json::from_str(JSON).unwrap();
        assert_eq!(r.version, 1);
        assert_eq!(open_items(&r), 3);
        assert_eq!(entries(&r), vec![Entry::Group(0), Entry::Group(1), Entry::Shared(0)]);
        assert_eq!(decision_text(&r.groups[0].tracks[0], &r.groups[0].files), "Unresolved");
        assert_eq!(music_dir(&r), PathBuf::from("/m"));
    }

    #[test]
    fn decision_text_names_the_file_number_and_reviews() {
        let r: Report = serde_json::from_str(JSON).unwrap();
        let mut t = r.groups[0].tracks[0].clone();
        t.decision = Some(Decision { action: "set".into(), file: Some("Mix/b.mp3".into()), stale: false, group_changed: false });
        assert_eq!(decision_text(&t, &r.groups[0].files), "-> 2");
        t.decision.as_mut().unwrap().group_changed = true;
        assert_eq!(decision_text(&t, &r.groups[0].files), "review: group changed");
        t.decision = Some(Decision { action: "none".into(), file: None, stale: false, group_changed: false });
        assert_eq!(decision_text(&t, &r.groups[0].files), "not owned");
    }

    #[test]
    fn action_command_lines() {
        let r: Report = serde_json::from_str(JSON).unwrap();
        let t = &r.groups[0].tracks[0];
        assert_eq!(args_set(t, "Mix/b.mp3"), ["versions", "set", "spotify", "spotify:track:x", "Mix/b.mp3"]);
        assert_eq!(args_none(t), ["versions", "none", "spotify", "spotify:track:x"]);
        assert_eq!(args_clear(t), ["versions", "clear", "spotify", "spotify:track:x"]);
        assert_eq!(args_label("Mix/a.mp3", "live"), ["versions", "label", "Mix/a.mp3", "live"]);
        assert_eq!(args_same("Mix/b.mp3", &["Mix/a.mp3".into()]), ["versions", "same", "Mix/b.mp3", "Mix/a.mp3"]);
        assert_eq!(args_shared_ok("mb:1"), ["versions", "shared-ok", "mb:1"]);
    }

    #[test]
    fn filter_keeps_order_and_unresolved_only() {
        let r: Report = serde_json::from_str(JSON).unwrap();
        assert_eq!(entries_filtered(&r, "", false), entries(&r));
        assert_eq!(entries_filtered(&r, "adagio", false), vec![Entry::Group(0)]);
        assert_eq!(entries_filtered(&r, "tiesto", false), vec![Entry::Group(0)]); // "Tiësto" folded
        assert_eq!(entries_filtered(&r, "", true), vec![Entry::Group(0), Entry::Shared(0)]);
        assert_eq!(entries_filtered(&r, "song", true), Vec::<Entry>::new());
    }

    #[test]
    fn reasons_read_as_minutes() {
        assert_eq!(
            human_reason("no live/remix/edit marker in the title; longest play 207 s, file 218 s; 443 s does not fit"),
            "no live/remix/edit marker in the title; longest listen 3:27, file 3:38; 7:23 does not fit"
        );
        assert_eq!(human_reason("title says live"), "title says live");
        assert_eq!(human_reason("12 songs"), "12 songs");
    }

    #[test]
    fn music_dir_from_the_tools_config() {
        assert_eq!(config_music_dir("contact = \"x\"\nmusic_dir = \"~/personal/music\"\n").as_deref(), Some("~/personal/music"));
        assert_eq!(config_music_dir("#music_dir = \"x\"\n"), None);
    }
}
