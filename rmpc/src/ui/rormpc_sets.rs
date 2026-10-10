//! rormpc: "+ set…" in the Hits / Play filter column (plans/combined-view.md "Sets as chips", phase 5): every tag
//! list, stored MPD playlist, followed Live playlist and smart list as a set, from `hits sets --json`.
//!
//! The picker has a section per kind with the sizes, `/` searches it (the menu's own search). A pick is added as
//! a `+` row under the fixed set rows (Space then cycles it off → + → − → off, "× clear sets" drops the added
//! rows). A smart list that cannot run (a cycle, made by a newer rormpc-tools) is listed with why, never added;
//! the open smart list is not offered as its own set.

use std::sync::{Arc, Mutex};

use crossbeam::channel::Sender;
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::{
        events::AppEvent,
        macros::{modal, status_error, status_info},
    },
    ui::{modals::menu::modal::MenuModal, rormpc_hits_rules},
};

/// One set of `hits sets --json` (rormpc-tools `hits_sets.listing`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct SetChoice {
    /// the canonical `--set` key: tag:NAME, playlist:NAME, live:ID, list:ID
    pub key: String,
    /// tag, playlist, live or list
    pub kind: String,
    pub name: String,
    /// its songs (None: not counted, e.g. a smart list never exported)
    #[serde(default)]
    pub songs: Option<usize>,
    /// why it cannot be used ("smart list cycle: A → B → A", a newer rormpc-tools)
    #[serde(default)]
    pub error: Option<String>,
}

impl SetChoice {
    /// Its name in the rows and the formula, as `hits` writes it in `set_names`: "Tag God", "Smart 80s party".
    pub fn label(&self) -> String {
        let kind = match self.kind.as_str() {
            "tag" => "Tag",
            "playlist" => "Playlist",
            "live" => "Live",
            "list" => "Smart",
            other => other,
        };
        format!("{kind} {}", self.name)
    }
}

#[derive(Debug, Deserialize)]
struct SetsFile {
    version: u32,
    sets: Vec<SetChoice>,
}

pub fn parse_sets(text: &str) -> Result<Vec<SetChoice>, String> {
    let file: SetsFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if file.version != 1 {
        return Err(format!("unsupported version {}", file.version));
    }
    Ok(file.sets)
}

/// The picker's state, shared with the thread that reads the sets and with the menu's items.
#[derive(Debug, Default)]
pub struct SetPicker {
    loading: bool,
    /// the sets read, for the pane to open the menu on its next frame
    ready: Option<Vec<SetChoice>>,
    /// the set picked in the menu, for the pane to add on its next frame
    picked: Option<SetChoice>,
}

/// Read the sets in the background (the MPD playlists' sizes take a round trip each); the menu opens when they
/// arrive (`take_ready`).
pub fn load(state: &Arc<Mutex<SetPicker>>, command: &[String], sender: Sender<AppEvent>) {
    let Ok(mut s) = state.lock() else { return };
    if s.loading {
        return;
    }
    s.loading = true;
    drop(s);
    status_info!("Reading tag lists, playlists, Live playlists and smart lists…");
    let (state, command) = (Arc::clone(state), command.to_vec());
    std::thread::spawn(move || {
        let read = crate::ui::rormpc_exceptions::run(&command, &["sets".to_owned(), "--json".to_owned()])
            .and_then(|out| parse_sets(&out));
        if let Ok(mut s) = state.lock() {
            s.loading = false;
            match read {
                Ok(sets) => s.ready = Some(sets),
                Err(err) => status_error!("hits sets: {err}"),
            }
        }
        let _ = sender.send(AppEvent::RequestRender);
    });
}

pub fn take_ready(state: &Arc<Mutex<SetPicker>>) -> Option<Vec<SetChoice>> {
    state.lock().ok()?.ready.take()
}

pub fn take_picked(state: &Arc<Mutex<SetPicker>>) -> Option<SetChoice> {
    state.lock().ok()?.picked.take()
}

const SECTIONS: [(&str, &str); 4] = [
    ("tag", "Tag lists"),
    ("playlist", "MPD playlists"),
    ("live", "Live playlists (YouTube)"),
    ("list", "Smart lists"),
];

/// A choice's line: "  God  (24)", "✓ God  (24)" when it is a row already, "! A · smart list cycle: A → B → A".
pub fn choice_line(c: &SetChoice, added: bool) -> String {
    if let Some(err) = &c.error {
        return format!("! {} · {err}", c.name);
    }
    let size = c.songs.map_or_else(String::new, |n| format!("  ({n})"));
    format!("{} {}{size}", if added { "✓" } else { " " }, c.name)
}

/// The "+ set…" menu: a section per kind. `added`: the keys with a row already (picking one sets it to +);
/// `open_list`: the smart list open in Play, which cannot be a set of itself.
pub fn open_picker(ctx: &Ctx, state: &Arc<Mutex<SetPicker>>, sets: Vec<SetChoice>, added: &[String], open_list: Option<&str>) {
    let sets: Vec<SetChoice> = sets
        .into_iter()
        .map(|mut c| {
            if c.error.is_none() && open_list.is_some_and(|id| c.key == format!("list:{id}")) {
                c.error = Some("open now: a smart list cannot be a set of itself".to_owned());
            }
            c
        })
        .collect();
    let title = if sets.is_empty() {
        "No tag lists, playlists, Live playlists or smart lists yet".to_owned()
    } else {
        format!("Add a set ({}) · / searches · Space on its row then cycles + − off", sets.len())
    };
    let sender = ctx.app_event_sender.clone();
    let mut menu = MenuModal::new(ctx).width(80).list_section(ctx, move |section| Some(section.item(title, |_| Ok(()))));
    for (kind, heading) in SECTIONS {
        let choices: Vec<SetChoice> = sets.iter().filter(|c| c.kind == kind).cloned().collect();
        if choices.is_empty() {
            continue;
        }
        let (state, sender, added) = (Arc::clone(state), sender.clone(), added.to_vec());
        menu = menu.list_section(ctx, move |mut section| {
            section.add_item(heading, |_| Ok(()));
            for c in choices {
                let line = choice_line(&c, added.contains(&c.key));
                let (state, sender) = (Arc::clone(&state), sender.clone());
                section.add_item(format!("  {line}"), move |_| {
                    if let Some(err) = &c.error {
                        status_error!("{}: {err}", c.label());
                        return Ok(());
                    }
                    rormpc_hits_rules::remember(&c.key, &c.label());
                    if let Ok(mut s) = state.lock() {
                        s.picked = Some(c.clone());
                    }
                    let _ = sender.send(AppEvent::RequestRender);
                    Ok(())
                });
            }
            Some(section)
        });
    }
    let menu = menu.list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(())))).build();
    modal!(ctx, menu);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape `hits sets --json` prints (rormpc-tools tests/test_hits_named_sets.py).
    #[test]
    fn parses_the_sets_json() {
        let json = r#"{"version": 1, "sets": [
            {"key": "tag:God", "kind": "tag", "name": "God", "songs": 2, "error": null},
            {"key": "playlist:Road: trip", "kind": "playlist", "name": "Road: trip", "songs": 3, "error": null},
            {"key": "live:yt-PL1", "kind": "live", "name": "Discover copy", "songs": 1, "error": null},
            {"key": "list:L1", "kind": "list", "name": "A", "songs": null, "error": "smart list cycle: A → B → A"}]}"#;
        let sets = parse_sets(json).expect("sets --json");
        assert_eq!(sets.iter().map(SetChoice::label).collect::<Vec<_>>(), [
            "Tag God",
            "Playlist Road: trip",
            "Live Discover copy",
            "Smart A"
        ]);
        assert_eq!(choice_line(&sets[0], false), "  God  (2)");
        assert_eq!(choice_line(&sets[1], true), "✓ Road: trip  (3)");
        assert_eq!(choice_line(&sets[3], false), "! A · smart list cycle: A → B → A");
        assert!(parse_sets(r#"{"version": 2, "sets": []}"#).is_err());
    }
}
