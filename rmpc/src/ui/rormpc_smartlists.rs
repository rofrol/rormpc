//! rormpc: smart lists in Play (plans/combined-view.md "Smart lists", phase 4): Play's filters saved under a
//! name by `hits lists` in the data repo (smartlists.jsonl).
//!
//! - `S` saves the filters on screen: a name prompt with the rules shown; a name that exists offers "Update NAME"
//!   or another name. The saved list becomes the open one.
//! - `L` opens the picker: Smart lists, Previous sources (the last applied rule sets, local state), then MPD
//!   playlists and Live playlists. Enter loads a list's rules into the filter column as a preview (it never plays
//!   by itself); `a` applies; `r` rename, `u` update with the filters on screen, `c` duplicate, `d` delete
//!   (confirmed; its exceptions go with it). The letters are matched by the actions they are bound to (Add,
//!   Rate, Update, ToggleConsume/ConsumeOff, Delete), as `a` is Apply in Play.
//! - The open list is part of the filters (`hits --open-list`): its own exceptions apply, and a new pin or
//!   exclusion defaults to it (`rormpc_exceptions::default_scope`).
//! - Apply of a list writes its MPD playlist "Smart NAME" (`hits lists export`); `musicdb update` does it hourly.
//!   A list made by a newer rormpc-tools is shown, never loaded.

use std::{
    collections::HashMap,
    fmt::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crossbeam::channel::Sender;
use rmpc_mpd::mpd_client::MpdClient;
use serde::{Deserialize, Serialize};

use crate::{
    config::keys::{CommonAction, GlobalAction, QueueActions},
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        macros::{modal, status_error, status_info},
    },
    ui::{
        modals::{
            confirm_modal::{Action, ConfirmModal},
            input_modal::InputModal,
            menu::modal::MenuModal,
        },
        rormpc_exceptions,
        rormpc_upnext,
    },
};

const HITS: &str = "hits";
/// Previous sources kept (decided with the plan: the last 10 applied rule sets).
const PREVIOUS_MAX: usize = 10;

/// A list's own exceptions (scope `list:ID`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Fixes {
    #[serde(default)]
    pub pins: u32,
    #[serde(default)]
    pub exclusions: u32,
}

/// One smart list of `hits lists --json`.
#[derive(Debug, Clone, Deserialize)]
pub struct SmartList {
    pub id: String,
    pub name: String,
    /// why this version cannot run it ("made by a newer rormpc-tools, update it (…)")
    #[serde(default)]
    pub blocked: Option<String>,
    /// its rules as a `hits` result's `args`: what the filter column loads
    #[serde(default)]
    pub args: Option<serde_json::Value>,
    #[serde(default)]
    pub exceptions: Fixes,
    /// when "Smart NAME" was last written, and its songs
    #[serde(default)]
    pub exported: Option<String>,
    #[serde(default)]
    pub exported_songs: usize,
}

#[derive(Debug, Deserialize)]
struct ListFile {
    version: u32,
    lists: Vec<SmartList>,
}

pub fn parse_lists(text: &str) -> Result<Vec<SmartList>, String> {
    let file: ListFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.version != 1 {
        return Err(format!("unsupported version {}", file.version));
    }
    Ok(file.lists)
}

fn cache() -> &'static Mutex<Vec<SmartList>> {
    static CACHE: OnceLock<Mutex<Vec<SmartList>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(Vec::new()))
}

/// `hits lists --json` (a local file read), kept for the names and rules Play shows.
pub fn load() -> Result<Vec<SmartList>, String> {
    let lists = rormpc_exceptions::run(&[HITS.to_owned()], &["lists".to_owned(), "--json".to_owned()])
        .and_then(|out| parse_lists(&out))?;
    if let Ok(mut c) = cache().lock() {
        c.clone_from(&lists);
    }
    Ok(lists)
}

/// Read the lists in the background once (Play's header names the open list by them).
pub fn load_in_background(sender: Sender<AppEvent>) {
    std::thread::spawn(move || {
        if load().is_ok() {
            let _ = sender.send(AppEvent::RequestRender);
        }
    });
}

/// A smart list by id, as last loaded.
pub fn cached(id: &str) -> Option<SmartList> {
    cache().lock().ok()?.iter().find(|l| l.id == id).cloned()
}

fn open_slot() -> &'static Mutex<Option<(String, String)>> {
    static OPEN: OnceLock<Mutex<Option<(String, String)>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(None))
}

/// Play's open smart list (id, name), set on every frame it draws.
pub fn set_open(open: Option<(String, String)>) {
    if let Ok(mut o) = open_slot().lock() {
        *o = open;
    }
}

pub fn open() -> Option<(String, String)> {
    open_slot().lock().ok()?.clone()
}

/// A list's name: the last load, else the open list's, else the start of its id.
pub fn name_of(id: &str) -> String {
    cached(id)
        .map(|l| l.name)
        .or_else(|| open().filter(|(i, _)| i == id).map(|(_, name)| name))
        .unwrap_or_else(|| id.chars().take(8).collect())
}

// ---------------------------------------------------------------- previous sources

/// One applied rule set (`$XDG_STATE_HOME/rormpc/previous-sources.json`, newest first): local state, not the data
/// repo, so "give me the old queue back" is one explicit Apply.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Previous {
    pub name: String,
    /// the `args` of the result that was applied
    pub rules: serde_json::Value,
    pub rules_hash: String,
    /// seconds since the epoch
    pub at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PreviousFile {
    #[serde(default)]
    sources: Vec<Previous>,
}

fn previous_path() -> PathBuf {
    let base = std::env::var("XDG_STATE_HOME").map_or_else(
        |_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".local/state"),
        PathBuf::from,
    );
    base.join("rormpc/previous-sources.json")
}

pub fn previous() -> Vec<Previous> {
    std::fs::read_to_string(previous_path())
        .ok()
        .and_then(|t| serde_json::from_str::<PreviousFile>(&t).ok())
        .map(|f| f.sources)
        .unwrap_or_default()
}

/// `new` first, an older entry with the same rules hash dropped, at most `PREVIOUS_MAX`.
fn remembered(mut sources: Vec<Previous>, new: Previous) -> Vec<Previous> {
    sources.retain(|p| p.rules_hash != new.rules_hash);
    sources.insert(0, new);
    sources.truncate(PREVIOUS_MAX);
    sources
}

/// After an Apply replaced the queue: remember the rules, and export the list it played (if one was open).
pub fn after_apply(name: &str, rules: &serde_json::Value, rules_hash: &str) {
    let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let new = Previous { name: name.to_owned(), rules: rules.clone(), rules_hash: rules_hash.to_owned(), at };
    let file = PreviousFile { sources: remembered(previous(), new) };
    let path = previous_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension("tmp");
    if let Ok(text) = serde_json::to_string_pretty(&file)
        && std::fs::write(&tmp, text).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path); // atomic: a crash never leaves half a file
    }
    if let Some(id) = rules.get("open_list").and_then(|v| v.as_str()).map(str::to_owned) {
        std::thread::spawn(move || {
            let args = ["lists".to_owned(), "export".to_owned(), id.clone()];
            if let Err(err) = rormpc_exceptions::run(&[HITS.to_owned()], &args) {
                status_error!("Smart list export of {}: {err}", name_of(&id));
            }
        });
    }
}

/// "today 21:14", "yesterday", "3 d ago" for a Unix time.
fn ago(at: u64, now: u64) -> String {
    let s = now.saturating_sub(at);
    match s {
        0..60 => "just now".to_owned(),
        60..3600 => format!("{} min ago", s / 60),
        3600..86_400 => format!("{} h ago", s / 3600),
        86_400..172_800 => "yesterday".to_owned(),
        _ => format!("{} d ago", s / 86_400),
    }
}

/// "exported 3 h ago" from an ISO local time (`hits lists --json`), or "not exported yet".
fn exported_label(list: &SmartList) -> String {
    let parsed = list.exported.as_deref().and_then(|t| {
        chrono::NaiveDateTime::parse_from_str(t, "%Y-%m-%dT%H:%M:%S").ok()?.and_local_timezone(chrono::Local).single()
    });
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match parsed {
        Some(t) => format!("{} songs · exported {}", list.exported_songs, ago(u64::try_from(t.timestamp()).unwrap_or(0), now)),
        None => "not exported yet".to_owned(),
    }
}

/// The picker's line of a list: "▶ 80s party · +2 ✚ −1 ⊘ · 287 songs · exported 3 h ago".
pub fn list_line(list: &SmartList, open: bool) -> String {
    let mut fixes = String::new();
    if list.exceptions.pins > 0 {
        let _ = write!(fixes, " · +{} ✚", list.exceptions.pins);
    }
    if list.exceptions.exclusions > 0 {
        let _ = write!(fixes, " · −{} ⊘", list.exceptions.exclusions);
    }
    format!("{} {}{fixes} · {}", if open { "▶" } else { " " }, list.name, exported_label(list))
}

// ---------------------------------------------------------------- what Play takes back

/// What the picker or the save modal hands back to Play, taken on its next frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    /// load these rules (a result's `args`) into the filters as a preview
    Load(serde_json::Value),
    /// load them and Apply once their preview is in
    Apply(serde_json::Value),
    /// keep the filters, leave the open list
    CloseList,
}

pub type Inbox = Arc<Mutex<Option<Pick>>>;

fn deliver(inbox: &Inbox, sender: &Sender<AppEvent>, pick: Pick) {
    if let Ok(mut slot) = inbox.lock() {
        *slot = Some(pick);
    }
    let _ = sender.send(AppEvent::RequestRender);
}

/// `hits lists ARGS` in the background: the status bar says how it went; `then` gets the lists read again.
fn run_lists(sender: Sender<AppEvent>, args: Vec<String>, ok: String, then: impl FnOnce(&[SmartList]) + Send + 'static) {
    std::thread::spawn(move || {
        let mut full = vec!["lists".to_owned()];
        full.extend(args);
        match rormpc_exceptions::run(&[HITS.to_owned()], &full) {
            Ok(_) => {
                status_info!("{ok}");
                rormpc_exceptions::invalidate_marks();
                match load() {
                    Ok(lists) => then(&lists),
                    Err(err) => status_error!("hits lists: {err}"),
                }
            }
            Err(err) => status_error!("hits lists: {err}"),
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

/// After a save or an update: the list becomes the open one (its rules loaded, as they are on screen).
fn open_by_name(inbox: Inbox, sender: Sender<AppEvent>, name: String) -> impl FnOnce(&[SmartList]) + Send + 'static {
    move |lists: &[SmartList]| {
        if let Some(args) = lists.iter().find(|l| l.name.eq_ignore_ascii_case(&name)).and_then(|l| l.args.clone()) {
            deliver(&inbox, &sender, Pick::Load(args));
        }
    }
}

// ---------------------------------------------------------------- S: save

/// What `S` needs from Play: the rules on screen as `hits` options and in words, and the open list.
#[derive(Debug, Clone)]
pub struct SaveRequest {
    pub rule_args: Vec<String>,
    pub lines: Vec<String>,
    /// "287 owned of 312 (the 25 missing stay in the rules)" when the preview is in
    pub counts: Option<String>,
    pub open: Option<(String, String)>,
}

/// `S`: the name prompt with the rules shown. A name that exists asks "Update NAME" or another name.
pub fn open_save(ctx: &Ctx, inbox: Inbox, req: SaveRequest) {
    let lists = match load() {
        Ok(lists) => lists,
        Err(err) => return status_error!("hits lists: {err}"),
    };
    let mut info = vec!["Save the filters as a smart list".to_owned()];
    info.extend(req.lines.iter().map(|l| format!("  {l}")));
    if let Some(counts) = &req.counts {
        info.push(format!("  Owned     {counts}"));
    }
    info.push("  New pins and exclusions go to the open list; library and set ones stay".to_owned());
    info.push("  Exported as the MPD playlist \"Smart NAME\" (Apply, and musicdb update hourly)".to_owned());
    let initial = req.open.as_ref().map(|(_, name)| name.clone()).unwrap_or_default();
    let sender = ctx.app_event_sender.clone();
    let menu = MenuModal::new(ctx)
        .width(84)
        .list_section(ctx, move |mut section| {
            for line in info {
                section.add_item(line, |_| Ok(()));
            }
            Some(section)
        })
        .input_section(ctx, "Name:", move |mut sect| {
            sect.add_initial_value(initial, ctx);
            sect.add_action(move |ctx, value| {
                let name = value.trim().to_owned();
                if name.is_empty() {
                    return status_info!("A smart list needs a name: nothing was saved");
                }
                if let Some(existing) = lists.iter().find(|l| l.name.eq_ignore_ascii_case(&name)) {
                    return confirm_update(ctx, inbox, existing.clone(), req);
                }
                let mut args = vec!["create".to_owned(), name.clone()];
                args.extend(req.rule_args);
                let then = open_by_name(inbox, sender.clone(), name.clone());
                run_lists(sender, args, format!("Smart list {name} saved"), then);
            });
            Some(sect)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

/// The name exists: update that list with the filters on screen, or pick another name.
fn confirm_update(ctx: &Ctx, inbox: Inbox, existing: SmartList, req: SaveRequest) {
    let message = vec![format!(
        "A smart list {} exists.\n\nUpdate it with the filters on screen, or save them under another name?",
        existing.name
    )];
    let (inbox2, req2) = (Arc::clone(&inbox), req.clone());
    let update = move |ctx: &Ctx| -> anyhow::Result<()> {
        update_list(ctx, inbox, &existing, req.rule_args);
        Ok(())
    };
    let other = move |ctx: &Ctx| -> anyhow::Result<()> {
        open_save(ctx, inbox2, SaveRequest { open: None, ..req2 });
        Ok(())
    };
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(message)
            .action(Action::CustomButtons {
                buttons: vec![
                    ("Cancel", Box::new(|_: &Ctx| Ok(()))),
                    ("Another name", Box::new(other)),
                    ("Update it", Box::new(update)),
                ],
            })
            .build()
    );
}

fn update_list(ctx: &Ctx, inbox: Inbox, list: &SmartList, rule_args: Vec<String>) {
    let sender = ctx.app_event_sender.clone();
    let mut args = vec!["update".to_owned(), list.id.clone()];
    args.extend(rule_args);
    let then = open_by_name(inbox, sender.clone(), list.name.clone());
    run_lists(sender, args, format!("Smart list {} updated with the filters on screen", list.name), then);
}

// ---------------------------------------------------------------- L: the picker

fn is_apply(e: &ActionEvent) -> bool {
    e.actions.iter().any(|a| matches!(a.as_common(), Some(CommonAction::AddOptions { .. })))
}

fn is_rename(e: &ActionEvent) -> bool {
    e.actions.iter().any(|a| matches!(a.as_common(), Some(CommonAction::Rate { .. } | CommonAction::Rename)))
}

fn is_update(e: &ActionEvent) -> bool {
    e.actions.iter().any(|a| matches!(a.as_global(), Some(GlobalAction::Update)))
}

fn is_duplicate(e: &ActionEvent) -> bool {
    e.actions.iter().any(|a| matches!(a.as_global(), Some(GlobalAction::ToggleConsume | GlobalAction::ToggleConsumeOnOff | GlobalAction::ConsumeOff)))
}

fn is_delete(e: &ActionEvent) -> bool {
    e.actions.iter().any(|a| matches!(a.as_queue(), Some(QueueActions::Delete)) || matches!(a.as_common(), Some(CommonAction::Delete)))
}

/// The previous source's rules as loadable `args`: a smart list that no longer exists is left out of them (its
/// exceptions went with it).
fn previous_args(p: &Previous, lists: &[SmartList]) -> serde_json::Value {
    let mut args = p.rules.clone();
    let gone = args.get("open_list").and_then(|v| v.as_str()).is_some_and(|id| !lists.iter().any(|l| l.id == id));
    if gone && let Some(obj) = args.as_object_mut() {
        obj.remove("open_list");
        obj.remove("open_list_name");
    }
    args
}

/// MPD's stored playlists with their sizes, the "Smart …" exports left out (each list's row shows its export),
/// split into (MPD playlists, Live playlists) by `liveplaylist list --json`'s names.
fn playlists(ctx: &Ctx) -> (Vec<(String, usize)>, Vec<(String, usize)>) {
    #[derive(Deserialize)]
    struct Sub {
        #[serde(default)]
        playlist: String,
    }
    #[derive(Deserialize)]
    struct Listing {
        #[serde(default)]
        subscriptions: Vec<Sub>,
    }
    let all = ctx
        .query_sync(|client| {
            let mut out = Vec::new();
            for pl in client.list_playlists()? {
                let n = client.list_playlist(&pl.name)?.0.len();
                out.push((pl.name, n));
            }
            Ok(out)
        })
        .unwrap_or_default();
    let live: Vec<String> = std::process::Command::new("liveplaylist")
        .args(["list", "--json"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| serde_json::from_slice::<Listing>(&o.stdout).ok())
        .map(|l| l.subscriptions.into_iter().map(|s| s.playlist).collect())
        .unwrap_or_default();
    let mut all: Vec<(String, usize)> = all.into_iter().filter(|(name, _)| !name.starts_with("Smart ")).collect();
    all.sort_by_key(|(name, _)| name.to_lowercase());
    all.into_iter().partition(|(name, _)| !live.contains(name))
}

/// `L`: smart lists, previous sources, MPD playlists and Live playlists. `rule_args`: the filters on screen,
/// for `u`.
pub fn open_picker(ctx: &Ctx, inbox: &Inbox, rule_args: Vec<String>, open: Option<&(String, String)>) {
    let lists = load().unwrap_or_else(|err| {
        status_error!("hits lists: {err}");
        Vec::new()
    });
    let previous = previous();
    let (mpd, live) = playlists(ctx);
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let open_id = open.as_ref().map(|(id, _)| id.clone());
    let (readable, blocked): (Vec<SmartList>, Vec<SmartList>) = lists.iter().cloned().partition(|l| l.blocked.is_none());
    let by_label: Arc<HashMap<String, SmartList>> =
        Arc::new(readable.iter().map(|l| (list_line(l, open_id.as_deref() == Some(l.id.as_str())), l.clone())).collect());
    let labels: Vec<String> = readable.iter().map(|l| list_line(l, open_id.as_deref() == Some(l.id.as_str()))).collect();
    let prev: Vec<(String, serde_json::Value)> = previous
        .iter()
        .map(|p| (format!("  {} · {}", p.name, ago(p.at, now)), previous_args(p, &lists)))
        .collect();
    let prev_by_label: Arc<HashMap<String, serde_json::Value>> = Arc::new(prev.iter().cloned().collect());
    let sender = ctx.app_event_sender.clone();
    let rule_args = Arc::new(rule_args);

    let mut menu = MenuModal::new(ctx).width(110).list_section(ctx, |mut section| {
        section.add_item("Lists · Enter load · a apply · r rename · u update · c duplicate · d delete · / search", |_| Ok(()));
        Some(section)
    });
    if let Some((_, name)) = open.cloned() {
        let (inbox, sender) = (Arc::clone(inbox), sender.clone());
        menu = menu.list_section(ctx, move |section| {
            Some(section.item(format!("× Close the smart list {name} (keep the filters)"), move |_| {
                deliver(&inbox, &sender, Pick::CloseList);
                Ok(())
            }))
        });
    }
    let heading = if lists.is_empty() { "Smart lists: none yet (S saves the filters as one)" } else { "Smart lists" };
    menu = menu.list_section(ctx, |section| Some(section.item(heading, |_| Ok(()))));
    if !labels.is_empty() {
        let (load, apply) = ((Arc::clone(inbox), sender.clone(), Arc::clone(&by_label)), (Arc::clone(inbox), sender.clone(), Arc::clone(&by_label)));
        let (ren, upd, dup, del) = (Arc::clone(&by_label), (Arc::clone(&by_label), Arc::clone(inbox), Arc::clone(&rule_args)), Arc::clone(&by_label), (Arc::clone(&by_label), Arc::clone(inbox), sender.clone()));
        menu = menu.multi_section(ctx, move |section| {
            let mut section = section
                .add_action("Load", move |_, label| {
                    if let Some(args) = load.2.get(&label).and_then(|l| l.args.clone()) {
                        deliver(&load.0, &load.1, Pick::Load(args));
                    }
                })
                .add_action_with_key("a Apply", is_apply, move |_, label| {
                    if let Some(args) = apply.2.get(&label).and_then(|l| l.args.clone()) {
                        deliver(&apply.0, &apply.1, Pick::Apply(args));
                    }
                })
                .add_action_with_key("r Rename", is_rename, move |ctx, label| {
                    if let Some(l) = ren.get(&label) {
                        rename(ctx, l.clone());
                    }
                })
                .add_action_with_key("u Update", is_update, move |ctx, label| {
                    if let Some(l) = upd.0.get(&label) {
                        update_list(ctx, Arc::clone(&upd.1), l, upd.2.as_ref().clone());
                    }
                })
                .add_action_with_key("c Copy", is_duplicate, move |ctx, label| {
                    if let Some(l) = dup.get(&label) {
                        duplicate(ctx, l.clone());
                    }
                })
                .add_action_with_key("d Delete", is_delete, move |ctx, label| {
                    if let Some(l) = del.0.get(&label) {
                        confirm_delete(ctx, Arc::clone(&del.1), l.clone());
                    }
                });
            for label in labels {
                section = section.add_item(label, ctx);
            }
            Some(section)
        });
    }
    if !blocked.is_empty() {
        menu = menu.list_section(ctx, move |mut section| {
            for l in blocked {
                let why = l.blocked.clone().unwrap_or_default();
                section.add_item(format!("! {} · {why}", l.name), move |_| {
                    status_error!("Smart list {}: {why}", l.name);
                    Ok(())
                });
            }
            Some(section)
        });
    }
    if !prev.is_empty() {
        menu = menu.list_section(ctx, |section| Some(section.item("Previous sources", |_| Ok(()))));
        let (load, apply) = ((Arc::clone(inbox), sender.clone(), Arc::clone(&prev_by_label)), (Arc::clone(inbox), sender.clone(), Arc::clone(&prev_by_label)));
        menu = menu.multi_section(ctx, move |section| {
            let mut section = section
                .add_action("Load", move |_, label| {
                    if let Some(args) = load.2.get(&label) {
                        deliver(&load.0, &load.1, Pick::Load(args.clone()));
                    }
                })
                .add_action_with_key("a Apply", is_apply, move |_, label| {
                    if let Some(args) = apply.2.get(&label) {
                        deliver(&apply.0, &apply.1, Pick::Apply(args.clone()));
                    }
                });
            for (label, _) in prev {
                section = section.add_item(label, ctx);
            }
            Some(section)
        });
    }
    for (title, items) in [("MPD playlists (Enter plays one: it replaces the queue)", mpd), ("Live playlists (YouTube)", live)] {
        if items.is_empty() {
            continue;
        }
        menu = menu.list_section(ctx, move |mut section| {
            section.add_item(title, |_| Ok(()));
            for (name, n) in items {
                section.add_item(format!("  {name}  ({n})"), move |ctx| {
                    rormpc_upnext::play_playlist(ctx, name.clone());
                    Ok(())
                });
            }
            Some(section)
        });
    }
    let menu = menu.list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(())))).build();
    modal!(ctx, menu);
}

fn rename(ctx: &Ctx, list: SmartList) {
    let sender = ctx.app_event_sender.clone();
    modal!(
        ctx,
        InputModal::new(ctx)
            .title("Rename the smart list")
            .input_label("Name:")
            .confirm_label("Rename")
            .initial_value(list.name.clone())
            .on_confirm(move |_, value| {
                let name = value.trim().to_owned();
                if !name.is_empty() && name != list.name {
                    let ok = format!("Smart list {} renamed to {name}", list.name);
                    run_lists(sender, vec!["rename".to_owned(), list.id, name], ok, |_| {});
                }
                Ok(())
            })
    );
}

fn duplicate(ctx: &Ctx, list: SmartList) {
    let sender = ctx.app_event_sender.clone();
    modal!(
        ctx,
        InputModal::new(ctx)
            .title("Duplicate the smart list (its rules and exceptions)")
            .input_label("Name:")
            .confirm_label("Duplicate")
            .initial_value(format!("{} copy", list.name))
            .on_confirm(move |_, value| {
                let name = value.trim().to_owned();
                if !name.is_empty() {
                    let ok = format!("Smart list {} duplicated as {name}", list.name);
                    run_lists(sender, vec!["duplicate".to_owned(), list.id, name], ok, |_| {});
                }
                Ok(())
            })
    );
}

fn confirm_delete(ctx: &Ctx, inbox: Inbox, list: SmartList) {
    let fixes = list.exceptions.pins + list.exceptions.exclusions;
    let message = vec![format!(
        "Delete the smart list {}?\n\n{}Its MPD playlist \"Smart {}\" goes too. The queue stays as it is.",
        list.name,
        if fixes > 0 { format!("Its {fixes} exceptions go with it. ") } else { String::new() },
        list.name
    )];
    let sender = ctx.app_event_sender.clone();
    let go = move |_: &Ctx| -> anyhow::Result<()> {
        let open = open().is_some_and(|(id, _)| id == list.id);
        let ok = format!("Smart list {} deleted", list.name);
        run_lists(sender.clone(), vec!["delete".to_owned(), list.id], ok, move |_| {
            if open {
                deliver(&inbox, &sender, Pick::CloseList);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `hits lists --json` prints (rormpc-tools tests/test_smartlists.py).
    #[test]
    fn parses_the_lists_json() {
        let json = r#"{"version": 1, "lists": [
            {"id": "L1", "name": "80s party", "rules": {"schema": 1}, "blocked": null,
             "args": {"period": "1985-1992", "top": "1-10", "genre": "+rock", "artist": "", "owned": false,
                      "rank": "billboard", "years_of": "chart", "sets": ["+billboard"], "show_excluded": false,
                      "source": null, "open_list": "L1", "open_list_name": "80s party"},
             "formula": "Billboard ∩ 1985-1992 ∩ Top 1-10% ∩ rock", "exceptions": {"pins": 2, "exclusions": 1},
             "playlist": "Smart 80s party", "exported": null, "exported_songs": 0,
             "created": "2026-10-10T10:00:00", "updated": "2026-10-10T10:00:00"},
            {"id": "L2", "name": "Polish 90s", "rules": {"schema": 2}, "blocked": "made by a newer rormpc-tools, update it (rules schema 2)",
             "args": null, "formula": null, "exceptions": {"pins": 0, "exclusions": 0}, "playlist": "Smart Polish 90s",
             "exported": "2026-10-10T09:00:00", "exported_songs": 96, "created": null, "updated": null}]}"#;
        let lists = parse_lists(json).expect("lists --json");
        assert_eq!(lists[0].exceptions, Fixes { pins: 2, exclusions: 1 });
        assert!(lists[0].args.as_ref().is_some_and(|a| a["open_list"] == "L1"));
        assert!(lists[1].blocked.as_deref().is_some_and(|b| b.starts_with("made by a newer rormpc-tools")));
        assert_eq!(list_line(&lists[0], true), "▶ 80s party · +2 ✚ · −1 ⊘ · not exported yet");
        assert!(parse_lists(r#"{"version": 2, "lists": []}"#).is_err());
    }

    fn prev(name: &str, hash: &str, at: u64) -> Previous {
        Previous { name: name.to_owned(), rules: serde_json::json!({}), rules_hash: hash.to_owned(), at }
    }

    #[test]
    fn previous_sources_keep_the_newest_ten_once_each() {
        let mut list = Vec::new();
        for i in 0..12 {
            list = remembered(list, prev(&format!("s{i}"), &format!("h{i}"), i));
        }
        assert_eq!(list.len(), 10);
        assert_eq!(list[0].name, "s11");
        assert_eq!(list[9].name, "s2");
        // the same rules again move up, not in twice
        let list = remembered(list, prev("again", "h5", 99));
        assert_eq!(list.len(), 10);
        assert_eq!(list[0].name, "again");
        assert_eq!(list.iter().filter(|p| p.rules_hash == "h5").count(), 1);
    }

    #[test]
    fn a_previous_source_of_a_deleted_list_loads_without_it() {
        let mut p = prev("x", "h", 0);
        p.rules = serde_json::json!({"period": null, "open_list": "gone", "open_list_name": "Gone"});
        let args = previous_args(&p, &[]);
        assert!(args.get("open_list").is_none() && args.get("open_list_name").is_none());
    }

    #[test]
    fn ages_in_words() {
        assert_eq!(ago(100, 130), "just now");
        assert_eq!(ago(0, 3 * 3600 + 5), "3 h ago");
        assert_eq!(ago(0, 90_000), "yesterday");
        assert_eq!(ago(0, 3 * 86_400), "3 d ago");
    }

    #[test]
    fn picker_keys_match_their_bound_actions() {
        let ev = |a: crate::shared::keys::ActionEvent| a;
        let of = |actions: Vec<_>| ev(crate::shared::keys::ActionEvent::from(Arc::new(actions)));
        assert!(is_update(&of(vec![GlobalAction::Update.into()])));
        assert!(is_duplicate(&of(vec![GlobalAction::ConsumeOff.into()])) && is_duplicate(&of(vec![GlobalAction::ToggleConsume.into()])));
        assert!(is_delete(&of(vec![QueueActions::Delete.into()])) && is_delete(&of(vec![CommonAction::Delete.into()])));
        assert!(is_rename(&of(vec![CommonAction::Rename.into()])));
        assert!(!is_update(&of(vec![QueueActions::Delete.into()])) && !is_apply(&of(vec![GlobalAction::Update.into()])));
    }
}
