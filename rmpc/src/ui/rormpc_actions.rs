//! rormpc: actions shared by context menus (Queue, Hits): trash a library file the same way as Ctrl-x,
//! set rmpc's like sticker, and show which key does the same thing outside the menu.

use std::process::Command;

use anyhow::Result;
use rmpc_mpd::mpd_client::MpdClient;

use crate::{
    config::keys::{CommonAction, GlobalAction},
    ctx::Ctx,
    shared::macros::modal,
    ui::modals::confirm_modal::{Action, ConfirmModal},
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

/// Ask, then `musicdb delete FILE` (Trash + deletion queue) in a background thread. Cancel is the default.
pub fn confirm_trash(ctx: &Ctx, file: String) {
    let undo = external_key_hint(ctx, &["musicdb", "undo"]);
    let message = vec![
        format!("Move this library file to the Trash?\n\n{file}"),
        format!("\nIts YouTube/ListenBrainz cleanup is queued for the deletion queue (ox).\nUndo:{undo}"),
    ];
    let on_trash = move |_: &Ctx| -> Result<()> {
        std::thread::spawn(move || {
            let _ = Command::new("musicdb").args(["delete", &file]).status();
        });
        Ok(())
    };
    modal!(
        ctx,
        ConfirmModal::builder()
            .ctx(ctx)
            .message(message)
            .action(Action::CustomButtons {
                buttons: vec![("Cancel", Box::new(|_: &Ctx| Ok(()))), ("Move to Trash", Box::new(on_trash))],
            })
            .build()
    );
}
