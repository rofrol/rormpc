//! rormpc: "Add to playlist…" from the Queue and Hits. Unlike upstream's save modal it shows which stored playlists
//! already have the songs: "✓" when all of them are there (Enter does nothing), "3/5" when some are (Enter adds
//! the missing ones). Adding never turns into removing. Membership is matched on the exact MPD URI.

use std::collections::HashSet;

use rmpc_mpd::mpd_client::MpdClient;

use crate::{
    ctx::Ctx,
    shared::{
        macros::{modal, status_error, status_info},
        mpd_client_ext::MpdClientExt as _,
    },
    ui::modals::menu::modal::MenuModal,
};

/// `files` are the songs to add (marked rows, else the cursor row), snapshotted now; `what` names them.
pub fn open_add_to_playlist(ctx: &Ctx, files: Vec<String>, what: String) {
    if files.is_empty() {
        return status_info!("No library song to add");
    }
    let lookup = ctx.query_sync(|client| {
        let mut out = Vec::new();
        for pl in client.list_playlists()? {
            let has: HashSet<String> = client.list_playlist(&pl.name)?.0.into_iter().collect();
            out.push((pl.name, has));
        }
        Ok(out)
    });
    let mut playlists = match lookup {
        Ok(p) => p,
        Err(err) => return status_error!("Cannot read the playlists: {err}"),
    };
    playlists.sort_by_key(|(name, _)| name.to_lowercase());
    let (new_files, total) = (files.clone(), files.len());
    let menu = MenuModal::new(ctx)
        .width(70)
        .input_section(ctx, "New playlist", move |mut sect| {
            sect.add_action(move |ctx, value| {
                if !value.is_empty() {
                    let files = new_files.clone();
                    ctx.command(move |_, client| {
                        client.create_playlist(&value, files)?;
                        status_info!("Created playlist {value}");
                        Ok(())
                    });
                }
            });
            Some(sect)
        })
        .list_section(ctx, move |mut section| {
            section.add_item(format!("Add {what} to:"), |_| Ok(()));
            for (name, has) in playlists {
                let missing: Vec<String> = files.iter().filter(|f| !has.contains(*f)).cloned().collect();
                let present = total - missing.len();
                let label = match (present, total) {
                    (p, t) if p == t => format!("✓ {name}  (already there)"),
                    (0, _) => format!("  {name}"),
                    (p, t) => format!("  {name}  ({p}/{t} there, adds {})", t - p),
                };
                section.add_item(label, move |ctx| {
                    if missing.is_empty() {
                        return Ok(()); // all present: nothing to do, and never a removal
                    }
                    let (name, n) = (name.clone(), missing.len());
                    let missing = missing.clone();
                    ctx.command(move |_, client| {
                        client.add_to_playlist_multiple(&name, missing)?;
                        status_info!("Added {n} to {name}");
                        Ok(())
                    });
                    Ok(())
                });
            }
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}
