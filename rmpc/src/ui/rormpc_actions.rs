//! rormpc: actions shared by context menus (Queue, Hits): delete a library file the same way as Ctrl-x,
//! set rmpc's like sticker, show which key does the same thing outside the menu, and keep the same file
//! from piling up in the queue.

use std::collections::HashSet;

use anyhow::Result;
use rmpc_mpd::{
    client::Client,
    commands::{Song, status::State},
    mpd_client::MpdClient,
};

use crate::{
    config::keys::{
        CommonAction,
        GlobalAction,
        actions::{AutoplayKind, Position},
    },
    ctx::Ctx,
    shared::{
        macros::{modal, status_info},
        mpd_client_ext::{Enqueue, MpdClientExt},
    },
    ui::modals::{
        confirm_modal::{Action, ConfirmModal},
        delete_menu::DeleteMenu,
    },
};

/// " (<C-x>)": the first global key bound to an external command whose arguments contain all `words`.
pub fn external_key_hint(ctx: &Ctx, words: &[&str]) -> String {
    let key = ctx.config.keybinds.global.iter().find_map(|(key, action)| match action {
        GlobalAction::ExternalCommand { command, .. }
            if words.iter().all(|w| command.iter().any(|c| c.contains(w))) =>
        {
            Some(key.to_string())
        }
        _ => None,
    });
    key.map(|k| format!("  ({k})")).unwrap_or_default()
}

/// " (r)": the navigation key bound to the Rate action (its menu has Like / Dislike / Neutral).
pub fn rate_key_hint(ctx: &Ctx) -> String {
    let key = ctx
        .config
        .keybinds
        .navigation
        .iter()
        .find_map(|(key, action)| matches!(action, CommonAction::Rate { .. }).then(|| key.to_string()));
    key.map(|k| format!("  ({k} menu)")).unwrap_or_default()
}

/// rmpc's like sticker: "2" like, "1" neutral, "0" dislike (musicdb syncs it to ListenBrainz).
pub fn set_like(ctx: &Ctx, file: String, value: &'static str) {
    ctx.command(move |_, client| {
        client.set_sticker(&file, "like", value)?;
        Ok(())
    });
}

/// The delete menu (Trash or permanent, keep or delete the history) for these library files.
pub fn open_delete_menu(ctx: &Ctx, files: Vec<String>) {
    modal!(ctx, DeleteMenu::new(ctx, files));
}

/// Ctrl-x is bound to `musicdb delete`: instead of running it, open the delete menu for the songs it would
/// get ($SELECTED_SONGS, else $FILE, the playing song). False for any other command, which runs as usual.
pub fn delete_menu_instead(ctx: &Ctx, command: &[String], env: &[(String, String)]) -> bool {
    let is_delete = matches!(command, [cmd, arg] if cmd.ends_with("musicdb") && arg == "delete");
    if !is_delete {
        return false;
    }
    let var = |name: &str| env.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str());
    let files: Vec<String> = var("SELECTED_SONGS")
        .or_else(|| var("FILE"))
        .map(|v| v.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect())
        .unwrap_or_default();
    if files.is_empty() {
        status_info!("No song to delete");
    } else {
        open_delete_menu(ctx, files);
    }
    true
}

/// Queue a library file from Hits. A file already in the queue is not appended again (unless `copy`):
/// Play plays the existing entry (the playing one first, so it is not restarted; a paused one resumes),
/// Add only says so. With random on a queue position tells nothing, so the message doesn't show one.
pub fn queue_file(ctx: &Ctx, path: String, play: bool, copy: bool) {
    let current = ctx.current_song().map(|s| s.id);
    let mut queued = ctx.queue.iter().filter(|s| s.file == path);
    let existing = queued.clone().find(|s| Some(s.id) == current).or_else(|| queued.next());
    if let (Some(song), false) = (existing, copy) {
        let (id, title) = (song.id, song_title(song));
        if !play {
            status_info!("Already in the queue: {title} (menu: Add another copy)");
        } else if Some(id) == current {
            if ctx.status.state == State::Pause {
                ctx.command(|_, client| Ok(client.play()?));
            }
        } else {
            ctx.command(move |_, client| Ok(client.play_id(id)?));
        }
        return;
    }
    Client::resolve_and_enqueue(
        ctx,
        vec![Enqueue::File { path }],
        Position::EndOfQueue,
        if play { AutoplayKind::First } else { AutoplayKind::None },
        ctx.current_song_index(),
        None,
    );
}

fn song_title(song: &Song) -> String {
    song.metadata.get("title").map_or_else(|| song.file.clone(), |t| t.last().to_owned())
}

/// Ids of queue entries whose file is queued more than once. Per file the playing entry stays, else the
/// first one by position.
pub fn duplicate_ids(queue: &[Song], current: Option<u32>) -> Vec<u32> {
    let playing: Option<&str> =
        queue.iter().find(|s| Some(s.id) == current).map(|s| s.file.as_str());
    let mut seen: HashSet<&str> = playing.into_iter().collect();
    queue
        .iter()
        .filter(|s| Some(s.id) != current && !seen.insert(s.file.as_str()))
        .map(|s| s.id)
        .collect()
}

/// Ask, then remove the duplicate queue entries. The queue is re-read when confirmed (other clients may
/// have changed it) and entries are deleted by id, since positions shift. Library files are untouched.
pub fn confirm_remove_duplicates(ctx: &Ctx, count: usize) {
    let message = vec![format!(
        "Remove {count} duplicate queue entries?\n\nThe same file queued more than once keeps one entry: \
         the playing one, else the first.\nLibrary files stay."
    )];
    let on_remove = |ctx: &Ctx| -> Result<()> {
        ctx.command(|_, client| {
            let current = client.get_current_song()?.map(|s| s.id);
            let queue = client.playlist_info()?.unwrap_or_default();
            let ids = duplicate_ids(&queue, current);
            for id in &ids {
                client.delete_id(*id)?;
            }
            status_info!("Removed {} duplicate queue entries", ids.len());
            Ok(())
        });
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
                    ("Remove", Box::new(on_remove))
                ],
            })
            .build()
    );
}

#[cfg(test)]
mod tests {
    use rmpc_mpd::commands::Song;

    use super::duplicate_ids;

    fn song(id: u32, file: &str) -> Song {
        Song { id, file: file.to_owned(), ..Default::default() }
    }

    #[test]
    fn keeps_the_playing_entry_else_the_first() {
        let q =
            [song(1, "a"), song(2, "b"), song(3, "a"), song(4, "a"), song(5, "b"), song(6, "c")];
        assert_eq!(duplicate_ids(&q, Some(3)), vec![1, 4, 5]);
        assert_eq!(duplicate_ids(&q, None), vec![3, 4, 5]);
        assert_eq!(duplicate_ids(&q, Some(6)), vec![3, 4, 5]);
        assert!(duplicate_ids(&[song(1, "a"), song(2, "b")], Some(1)).is_empty());
    }
}
