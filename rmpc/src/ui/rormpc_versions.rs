//! rormpc: which library files have other versions (a name several owned files share, `musicdb versions`), for
//! the Queue's `≋` column and "Find versions…". Membership is read once in the background from
//! `musicdb versions --json --all` and cached; it is read again after a deletion, a merge or a library update,
//! never per row. "Find versions…" opens the Versions tab on the file's group and remembers where it came from.

use std::{
    collections::HashSet,
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::Deserialize;

use crate::{
    config::{tabs::{PaneType, TabName}, keys::QueueActions},
    ctx::Ctx,
    shared::{
        events::AppEvent,
        macros::{status_error, status_info},
    },
    ui::UiAppEvent,
};

/// The Queue column's mark: the song's group has more than one owned file.
pub const MARK: &str = "≋";

#[derive(Debug, Default)]
struct Members {
    files: HashSet<String>,
    loaded: bool,
    loading: bool,
    /// asked again while a read ran: its answer may predate the change, so read once more
    again: bool,
}

static MEMBERS: LazyLock<Mutex<Members>> = LazyLock::new(Mutex::default);

#[derive(Deserialize)]
struct Report {
    #[serde(default)]
    groups: Vec<Group>,
}

#[derive(Deserialize)]
struct Group {
    #[serde(default)]
    files: Vec<GroupFile>,
}

#[derive(Deserialize)]
struct GroupFile {
    file: String,
}

/// Library files in a group of several owned files, from `musicdb versions --json [--all]`.
fn parse(out: &str) -> Result<HashSet<String>, String> {
    let r: Report = serde_json::from_str(out).map_err(|e| format!("musicdb versions: {e}"))?;
    Ok(r.groups.into_iter().filter(|g| g.files.len() > 1).flat_map(|g| g.files).map(|f| f.file).collect())
}

/// Has this library file other versions? False until the first read is done.
pub fn has_versions(file: &str) -> bool {
    MEMBERS.lock().is_ok_and(|m| m.files.contains(file))
}

/// The Versions column of a song.
pub fn marker(file: &str) -> Option<&'static str> {
    has_versions(file).then_some(MARK)
}

/// Replace the cache with a fresh full report (the Versions pane reads the same JSON).
pub fn set_files(files: HashSet<String>) {
    if let Ok(mut m) = MEMBERS.lock() {
        m.files = files;
        m.loaded = true;
    }
}

/// Read the membership again in the background; a call while a read runs makes it read once more after it.
pub fn refresh(sender: &crossbeam::channel::Sender<AppEvent>) {
    {
        let Ok(mut m) = MEMBERS.lock() else { return };
        if m.loading {
            m.again = true;
            return;
        }
        m.loading = true;
    }
    let sender = sender.clone();
    std::thread::spawn(move || {
        loop {
            let out = std::process::Command::new("musicdb").args(["versions", "--json", "--all"]).output();
            let result = match out {
                Ok(o) if o.status.success() => parse(&String::from_utf8_lossy(&o.stdout)),
                Ok(o) => Err(String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or("failed").to_owned()),
                Err(err) => Err(crate::shared::dependencies::cannot_run("musicdb", &err)),
            };
            let Ok(mut m) = MEMBERS.lock() else { return };
            match result {
                Ok(files) => {
                    m.files = files;
                    m.loaded = true;
                }
                // the marks are a hint: keep the old ones and say why they may be stale
                Err(err) => log::warn!("versions marks not refreshed: {err}"),
            }
            if !std::mem::take(&mut m.again) {
                m.loading = false;
                break;
            }
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Counts `changed` calls: the Versions pane reloads when it differs from the one it loaded for.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// After a deletion, a merge or a library update: the Versions pane reloads, and the marks are read again
/// (unless nothing has asked for them yet).
pub fn changed(sender: &crossbeam::channel::Sender<AppEvent>) {
    GENERATION.fetch_add(1, Ordering::Relaxed);
    let used = MEMBERS.lock().is_ok_and(|m| m.loaded || m.loading);
    if used {
        refresh(sender);
    }
    let _ = sender.send(AppEvent::RequestRender);
}

/// The first read, when nothing was read yet.
pub fn ensure_loaded(sender: &crossbeam::channel::Sender<AppEvent>) {
    let start = MEMBERS.lock().is_ok_and(|m| !m.loaded && !m.loading);
    if start {
        refresh(sender);
    }
}

/// Where "Find versions…" came from, and the file to select.
#[derive(Debug, Clone)]
pub struct Jump {
    pub file: String,
    pub back: TabName,
}

static JUMP: LazyLock<Mutex<Option<Jump>>> = LazyLock::new(Mutex::default);
/// The Queue's cursor (song id) and scroll when it left for Versions, restored when Versions goes back.
static QUEUE_VIEW: LazyLock<Mutex<Option<(u32, usize)>>> = LazyLock::new(Mutex::default);

/// The jump the Versions pane is to show, once.
pub fn take_jump() -> Option<Jump> {
    JUMP.lock().ok().and_then(|mut j| j.take())
}

/// The Queue view to restore, once.
pub fn take_queue_view() -> Option<(u32, usize)> {
    QUEUE_VIEW.lock().ok().and_then(|mut v| v.take())
}

/// Versions left some other way than back: the Queue keeps its usual behaviour.
pub fn forget_queue_view() {
    if let Ok(mut v) = QUEUE_VIEW.lock() {
        *v = None;
    }
}

/// Versions' Back / Esc after a jump: the tab it came from, its view restored there.
pub fn go_back(ctx: &Ctx, tab: TabName) {
    let _ = ctx.app_event_sender.send(AppEvent::UiAppEvent(UiAppEvent::ChangeTab(tab)));
}

/// The tab holding the Versions pane.
fn versions_tab(ctx: &Ctx) -> Option<TabName> {
    ctx.config.tabs.names.iter().find(|name| {
        ctx.config.tabs.tabs.get(*name).is_some_and(|t| t.panes.panes_iter().any(|p| matches!(p.pane, PaneType::Versions)))
    }).cloned()
}

/// " (V)": the Queue key bound to `FindVersions`.
pub fn key_hint(ctx: &Ctx) -> String {
    let key = ctx.config.keybinds.queue.iter().find_map(|(key, action)| {
        matches!(action, QueueActions::FindVersions).then(|| key.to_string())
    });
    key.map(|k| format!("  ({k})")).unwrap_or_default()
}

/// "Find versions…": open the Versions tab on the file's group, the file selected. False when it did not go
/// (no other versions known, or no Versions tab).
pub fn find_versions(ctx: &Ctx, file: &str, queue_view: Option<(u32, usize)>) -> bool {
    let known = MEMBERS.lock().is_ok_and(|m| m.loaded);
    if known && !has_versions(file) {
        status_info!("No other versions of this song in the library");
        return false;
    }
    let Some(tab) = versions_tab(ctx) else {
        status_error!("No tab has the Versions pane (see RORMPC.md)");
        return false;
    };
    if let Ok(mut j) = JUMP.lock() {
        *j = Some(Jump { file: file.to_owned(), back: ctx.active_tab.clone() });
    }
    if let Ok(mut v) = QUEUE_VIEW.lock() {
        *v = queue_view;
    }
    let _ = ctx.app_event_sender.send(AppEvent::UiAppEvent(UiAppEvent::ChangeTab(tab)));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn members_are_the_files_of_groups_with_several_files() {
        let json = r#"{"version": 1, "groups": [
            {"name": "a|b", "files": [{"file": "x.mp3"}, {"file": "y.flac"}], "tracks": [], "pending": []},
            {"name": "c|d", "files": [{"file": "z.mp3"}], "tracks": [], "pending": []}], "shared": []}"#;
        let files = parse(json).expect("test report");
        assert_eq!(files, HashSet::from(["x.mp3".to_owned(), "y.flac".to_owned()]));
        assert!(parse("usage: musicdb").is_err());
    }
}
