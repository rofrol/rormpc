//! rormpc: exceptions to the Hits rules (plans/combined-view.md, phase 2), kept by `hits except` in the data repo.
//!
//! - Pin ✚ (`+` on a Hits or Queue row): the song is in, whatever the filters say. It needs an owned file, has no
//!   rank and sits after the ranked rows.
//! - Exclusion ⊘ (`-`): the song is out. Any applicable exclusion beats any pin; a pin beats `-` sets, genres
//!   and artists.
//! - Scope: library (every result) or one set, which applies only while that set is `+`. `+` / `-` ask for it in
//!   a small menu, the default first.
//!
//! A plain "Remove from queue" or a song added by hand stays a one-off: only `+` / `-` record an exception. The
//! queue itself never changes here; the next Apply or "Play these" takes the exception into account. The
//! Exceptions list (Hits filter column) shows every exception with `hits hide` included and removes one on Enter.

use std::{process::Command, sync::Arc};

use serde::Deserialize;

use crate::{
    ctx::Ctx,
    config::keys::QueueActions,
    shared::macros::{modal, status_error, status_info},
    ui::{modals::menu::modal::MenuModal, rormpc_hits_rules::SETS},
};

/// Runs after an exception was recorded or removed (the Hits pane runs `hits` again to refresh its marks).
pub type Done = Arc<dyn Fn() + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Pin,
    Exclude,
}

impl Kind {
    fn verb(self) -> &'static str {
        match self {
            Kind::Pin => "pin",
            Kind::Exclude => "exclude",
        }
    }

    pub fn mark(self) -> &'static str {
        match self {
            Kind::Pin => "✚",
            Kind::Exclude => "⊘",
        }
    }
}

/// The song an exception is about: an owned file, else (a missing chart row) its chart key.
#[derive(Debug, Clone)]
pub struct Target {
    pub file: Option<String>,
    pub chart_key: Option<String>,
    pub artist: String,
    pub title: String,
}

/// One exception on a Hits row (`hits --json` rows' "exceptions").
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct RowException {
    pub action: String,
    pub scope: String,
    #[serde(default)]
    pub applies: bool,
    /// "hide": read from `hits hide`'s log
    #[serde(default)]
    pub via: Option<String>,
}

/// One exception of `hits exceptions --json`.
#[derive(Debug, Clone, Deserialize)]
pub struct Listed {
    pub action: String,
    pub scope: String,
    #[serde(default)]
    pub song: Option<String>,
    #[serde(default)]
    pub chart_key: Option<String>,
    #[serde(default)]
    pub artist: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub file: Option<String>,
    /// a pinned song whose file is gone: skipped, never dropped from the log
    #[serde(default)]
    pub gone: bool,
    #[serde(default)]
    pub via: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ListFile {
    version: u32,
    exceptions: Vec<Listed>,
}

/// "library" -> "Library", "set:billboard" -> "Billboard US" (the chip's label).
pub fn scope_label(scope: &str) -> String {
    match scope.strip_prefix("set:") {
        _ if scope == "library" => "Library".to_owned(),
        Some(key) => SETS.iter().find(|(k, _, _)| *k == key).map_or_else(|| key.to_owned(), |(_, label, _)| (*label).to_owned()),
        None => scope.to_owned(),
    }
}

/// "pinned · library", "excluded · Billboard US (hits hide)", "excluded · my likes (only while it is +)".
pub fn describe(e: &RowException) -> String {
    let what = if e.action == "pin" { "pinned" } else { "excluded" };
    let scope = if e.scope == "library" { "library".to_owned() } else { scope_label(&e.scope) };
    let hide = if e.via.as_deref() == Some("hide") { " (hits hide)" } else { "" };
    let idle = if e.applies { "" } else { " — not now: only while that set is +" };
    format!("{what} · {scope}{hide}{idle}")
}

/// The default scope of a new pin or exclusion: the open smart list, else library (decided 2026-10-09). Smart
/// lists come later (phase 4); until then it is always library.
pub fn default_scope() -> String {
    "library".to_owned()
}

/// The scopes `+` / `-` offer: the default first, then library and each `+` set of the selection.
pub fn scopes(plus_sets: [i8; 4]) -> Vec<String> {
    let mut out = vec![default_scope()];
    let rest = std::iter::once("library".to_owned())
        .chain(SETS.iter().zip(plus_sets).filter(|(_, s)| *s > 0).map(|((key, _, _), _)| format!("set:{key}")));
    for scope in rest {
        if !out.contains(&scope) {
            out.push(scope);
        }
    }
    out
}

fn scope_item(kind: Kind, scope: &str) -> String {
    match (kind, scope) {
        (Kind::Pin, "library") => "in every list (library)".to_owned(),
        (Kind::Exclude, "library") => "out of every list (library)".to_owned(),
        (Kind::Pin, s) => format!("in {} only (while it is +)", scope_label(s)),
        (Kind::Exclude, s) => format!("out of {} (while it is +)", scope_label(s)),
    }
}

/// `hits` with these arguments; Err is stderr's last line.
fn run(command: &[String], args: &[String]) -> Result<String, String> {
    match Command::new(&command[0]).args(&command[1..]).args(args).output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("hits failed")
            .to_owned()),
        Err(err) => Err(crate::shared::dependencies::cannot_run(&command[0], &err)),
    }
}

/// `hits except …` in the background; the result goes to the status bar, then `done`.
fn run_in_background(command: Vec<String>, args: Vec<String>, done: Option<Done>, ok: String) {
    std::thread::spawn(move || match run(&command, &args) {
        Ok(_) => {
            status_info!("{ok}");
            if let Some(done) = done {
                done();
            }
        }
        Err(err) => status_error!("hits except: {err}"),
    });
}

/// The `hits except` arguments that name the song.
fn target_args(target: &Target) -> Vec<String> {
    let mut args = match (&target.file, &target.chart_key) {
        (Some(file), _) => vec!["--file".to_owned(), file.clone()],
        (None, Some(key)) => vec!["--chart-key".to_owned(), key.clone()],
        (None, None) => Vec::new(),
    };
    args.extend(["--artist".to_owned(), target.artist.clone(), "--title".to_owned(), target.title.clone()]);
    args
}

/// `+` / `-` on a row: the scope menu, then `hits except pin|exclude`. A pin needs an owned file.
pub fn open_scope_menu(ctx: &Ctx, command: Vec<String>, kind: Kind, target: Target, plus_sets: [i8; 4], done: Option<Done>) {
    if kind == Kind::Pin && target.file.is_none() {
        return status_info!("A pin needs an owned file: '{}' is missing (fetch it first, or exclude it)", target.title);
    }
    if target.file.is_none() && target.chart_key.is_none() {
        return status_error!("No song to {} here", kind.verb());
    }
    let what = if kind == Kind::Pin { "Pin" } else { "Exclude" };
    let title = format!("{what} '{}' ({})", target.title, target.artist);
    let menu = MenuModal::new(ctx)
        .width(60)
        .list_section(ctx, move |mut section| {
            section.add_item(title, |_| Ok(()));
            for scope in scopes(plus_sets) {
                let (command, done) = (command.clone(), done.clone());
                let mut args = vec!["except".to_owned(), kind.verb().to_owned(), "--scope".to_owned(), scope.clone()];
                args.extend(target_args(&target));
                let ok = format!("{} {} · {}: '{}'", kind.mark(), if kind == Kind::Pin { "pinned" } else { "excluded" },
                    scope_label(&scope), target.title);
                section.add_item(scope_item(kind, &scope), move |_| {
                    run_in_background(command, args, done, ok);
                    Ok(())
                });
            }
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

/// The exceptions grouped by scope (library first, then the sets in chip order, then anything else), as the
/// list shows them.
pub fn grouped(mut items: Vec<Listed>) -> Vec<(String, Vec<Listed>)> {
    let order = |scope: &str| -> usize {
        if scope == "library" {
            return 0;
        }
        scope.strip_prefix("set:").and_then(|k| SETS.iter().position(|(key, _, _)| *key == k)).map_or(99, |i| i + 1)
    };
    items.sort_by(|a, b| {
        (order(&a.scope), &a.scope, &a.action, a.artist.as_deref().unwrap_or_default().to_lowercase())
            .cmp(&(order(&b.scope), &b.scope, &b.action, b.artist.as_deref().unwrap_or_default().to_lowercase()))
    });
    let mut out: Vec<(String, Vec<Listed>)> = Vec::new();
    for e in items {
        match out.last_mut() {
            Some((scope, list)) if *scope == e.scope => list.push(e),
            _ => out.push((e.scope.clone(), vec![e])),
        }
    }
    out
}

/// "✚ Kombi · Black and White", "! ✚ Maanam · Kocham cię (file gone)", "⊘ Toto · Africa (hits hide)".
pub fn list_line(e: &Listed) -> String {
    let mark = if e.action == "pin" { "✚" } else { "⊘" };
    let name = match (&e.artist, &e.title) {
        (Some(a), Some(t)) => format!("{a} · {t}"),
        (None, Some(t)) => t.clone(),
        _ => e.file.clone().or_else(|| e.chart_key.clone()).or_else(|| e.song.clone()).unwrap_or_default(),
    };
    let note = if e.gone { " (file gone)" } else if e.via.as_deref() == Some("hide") { " (hits hide)" } else { "" };
    format!("{}{mark} {name}{note}", if e.gone { "! " } else { "" })
}

/// The Exceptions list: every pin and exclusion by scope; Enter removes the one under the cursor (a hide is
/// unhidden). Reading it is a local file read (`hits exceptions --json`), so the menu opens right away.
pub fn open_list(ctx: &Ctx, command: &[String], done: Option<&Done>) {
    let listed = run(command, &["exceptions".to_owned(), "--json".to_owned()])
        .and_then(|out| serde_json::from_str::<ListFile>(&out).map_err(|e| e.to_string()))
        .and_then(|f| if f.version == 1 { Ok(f.exceptions) } else { Err(format!("unsupported version {}", f.version)) });
    let items = match listed {
        Ok(items) => items,
        Err(err) => return status_error!("hits exceptions: {err}"),
    };
    if items.is_empty() {
        return status_info!("No exceptions yet: + pins a row, - excludes it");
    }
    let mut menu = MenuModal::new(ctx).width(70);
    let title = format!("Exceptions ({}) · Enter removes one · / searches", items.len());
    menu = menu.list_section(ctx, move |section| Some(section.item(title, |_| Ok(()))));
    for (scope, list) in grouped(items) {
        let (command, done) = (command.to_vec(), done.cloned());
        menu = menu.list_section(ctx, move |mut section| {
            section.add_item(scope_label(&scope), |_| Ok(()));
            for e in list {
                let mut args = vec!["except".to_owned(), "remove".to_owned(), "--scope".to_owned(), e.scope.clone()];
                match (&e.song, &e.chart_key) {
                    (Some(id), _) => args.extend(["--id".to_owned(), id.clone()]),
                    (None, Some(key)) => args.extend(["--chart-key".to_owned(), key.clone()]),
                    (None, None) => continue,
                }
                let line = list_line(&e);
                let ok = format!("Exception removed: {line}");
                let (command, done) = (command.clone(), done.clone());
                section.add_item(format!("  {line}"), move |_| {
                    run_in_background(command, args, done, ok);
                    Ok(())
                });
            }
            Some(section)
        });
    }
    let menu = menu.list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(())))).build();
    modal!(ctx, menu);
}

/// "  (+)" after a menu item: the key bound to the action, if any.
pub fn key_hint(ctx: &Ctx, kind: Kind) -> String {
    let want = |a: &QueueActions| match kind {
        Kind::Pin => matches!(a, QueueActions::PinSong),
        Kind::Exclude => matches!(a, QueueActions::ExcludeSong),
    };
    crate::ui::rormpc_filter::binding(&ctx.config.keybinds.queue, want).map(|k| format!("  ({k})")).unwrap_or_default()
}

/// The `+` sets of the Hits result on screen (its file's `args.sets`), for the Queue's scope menu: the songs in
/// the queue came from it when it was played as the source.
pub fn applied_plus_sets(path: &str) -> [i8; 4] {
    #[derive(Deserialize)]
    struct Args {
        #[serde(default)]
        sets: Option<Vec<String>>,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        sort: Option<String>,
    }
    #[derive(Deserialize)]
    struct File {
        args: Args,
    }
    let path = match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_owned(),
    };
    let Some(file) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<File>(&t).ok()) else {
        return [0; 4];
    };
    let sets = match &file.args.sets {
        Some(values) => crate::ui::rormpc_hits_rules::parse_sets(values),
        None => crate::ui::rormpc_hits_rules::from_source(file.args.source.as_deref(), file.args.sort.as_deref()).0,
    };
    sets.map(|s| s.max(0))
}

/// `+` / `-` on a Queue row: the song's file, the `+` sets of the Hits result the queue may have come from.
pub fn open_for_queue_song(ctx: &Ctx, kind: Kind, song: &rmpc_mpd::commands::Song) {
    let tag = |name: &str| song.metadata.get(name).map(|v| v.last().to_owned());
    let target = Target {
        file: Some(song.file.clone()),
        chart_key: None,
        artist: tag("artist").unwrap_or_default(),
        title: tag("title").unwrap_or_else(|| song.file.clone()),
    };
    let plus = applied_plus_sets(HITS_FILE);
    open_scope_menu(ctx, vec![HITS.to_owned()], kind, target, plus, None);
}

/// The `hits` program and result file the Queue uses (the Hits pane's defaults, `config/tabs.rs`).
const HITS: &str = "hits";
const HITS_FILE: &str = "~/.cache/rormpc/hits/current.json";

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(action: &str, scope: &str, artist: &str) -> Listed {
        Listed {
            action: action.to_owned(),
            scope: scope.to_owned(),
            song: Some(format!("id-{artist}")),
            chart_key: None,
            artist: Some(artist.to_owned()),
            title: Some("t".to_owned()),
            file: Some("f".to_owned()),
            gone: false,
            via: None,
        }
    }

    #[test]
    fn scopes_put_the_default_first_then_the_plus_sets() {
        assert_eq!(scopes([1, 0, -1, 1]), ["library", "set:billboard", "set:recommended"]);
        assert_eq!(scopes([0; 4]), ["library"]);
        assert_eq!(scope_label("set:likes"), "my likes");
        assert_eq!(scope_label("library"), "Library");
    }

    #[test]
    fn row_exceptions_describe_their_scope_and_whether_they_apply() {
        let hide = RowException { action: "exclude".into(), scope: "set:billboard".into(), applies: true, via: Some("hide".into()) };
        assert_eq!(describe(&hide), "excluded · Billboard US (hits hide)");
        let idle = RowException { action: "pin".into(), scope: "set:likes".into(), applies: false, via: None };
        assert_eq!(describe(&idle), "pinned · my likes — not now: only while that set is +");
    }

    #[test]
    fn the_list_groups_by_scope_library_first() {
        let mut gone = listed("pin", "library", "Maanam");
        gone.gone = true;
        let items = vec![listed("exclude", "set:billboard", "Toto"), listed("pin", "library", "Kombi"), gone,
            listed("exclude", "library", "Crazy Frog")];
        let groups = grouped(items);
        let shape: Vec<(&str, Vec<String>)> =
            groups.iter().map(|(s, l)| (s.as_str(), l.iter().map(list_line).collect())).collect();
        assert_eq!(shape, [
            ("library", vec!["⊘ Crazy Frog · t".to_owned(), "✚ Kombi · t".to_owned(), "! ✚ Maanam · t (file gone)".to_owned()]),
            ("set:billboard", vec!["⊘ Toto · t".to_owned()]),
        ]);
    }

    #[test]
    fn list_file_contract() {
        // `hits exceptions --json` (rormpc-tools hits_exceptions.listing)
        let text = r#"{"version": 1, "exceptions": [{"id": "e1", "action": "exclude", "scope": "set:billboard",
            "song": null, "chart_key": "toto|africa", "artist": "Toto", "title": "Africa", "file": null,
            "gone": false, "via": "hide", "ts": "2026-10-10T01:00:00"}]}"#;
        let f: ListFile = serde_json::from_str(text).unwrap();
        assert_eq!((f.version, f.exceptions[0].chart_key.as_deref()), (1, Some("toto|africa")));
        assert_eq!(list_line(&f.exceptions[0]), "⊘ Toto · Africa (hits hide)");
    }
}
