//! rormpc: "Tags…" for a library song: my hand-made lists (`musicdb tag`, e.g. God, melancholic, tearjerkers)
//! with a ✓ on the ones the song is on, toggled with Enter, a new list by name, and corrections to the song's
//! MusicBrainz genres (`musicdb genre add|exclude|reset`). The logs live in the data repo; files are not changed.

use std::process::Command;

use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::macros::{modal, status_error, status_info},
    ui::modals::{input_modal::InputModal, menu::modal::MenuModal},
};

const MUSICDB: &str = "musicdb";

#[derive(Debug, Deserialize)]
struct TagsOf {
    lists: Vec<String>,
    all_lists: Vec<String>,
    genres_added: Vec<String>,
    genres_excluded: Vec<String>,
}

fn run(args: &[String]) -> Result<String, String> {
    match Command::new(MUSICDB).args(args).output() {
        Ok(out) if out.status.success() => Ok(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => Err(String::from_utf8_lossy(&out.stderr)
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("musicdb failed")
            .to_owned()),
        Err(err) => Err(crate::shared::dependencies::cannot_run(MUSICDB, &err)),
    }
}

/// `musicdb <args>` in the background; its last line goes to the status bar.
fn run_in_background(args: Vec<String>) {
    std::thread::spawn(move || match run(&args) {
        Ok(out) => status_info!("{}", out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("done")),
        Err(err) => status_error!("musicdb: {err}"),
    });
}

/// Ask for a name, then run `musicdb <verb…> NAME -- FILE`.
fn ask(ctx: &Ctx, title: &'static str, label: &'static str, verb: Vec<&'static str>, file: String) {
    modal!(
        ctx,
        InputModal::new(ctx).title(title).input_label(label).confirm_label("OK").on_confirm(move |_, value| {
            if !value.trim().is_empty() {
                let mut args: Vec<String> = verb.iter().map(|s| (*s).to_owned()).collect();
                args.extend([value.trim().to_owned(), "--".to_owned(), file.clone()]);
                run_in_background(args);
            }
            Ok(())
        })
    );
}

/// The Tags menu for one library file. Reading its lists is a local file read (`musicdb tag of`), so the menu
/// opens right away.
pub fn open_tags_menu(ctx: &Ctx, file: String) {
    let of = run(&["tag".into(), "of".into(), "--json".into(), file.clone()])
        .and_then(|out| serde_json::from_str::<TagsOf>(&out).map_err(|e| e.to_string()));
    let of = match of {
        Ok(of) => of,
        Err(err) => return status_error!("musicdb tag: {err}"),
    };
    let (f1, f2, f3, f4) = (file.clone(), file.clone(), file.clone(), file.clone());
    let menu = MenuModal::new(ctx)
        .width(50)
        .list_section(ctx, move |mut section| {
            for name in &of.all_lists {
                let on = of.lists.contains(name);
                let (file, name) = (f1.clone(), name.clone());
                section.add_item(format!("{} {name}", if on { "✓" } else { " " }), move |_| {
                    let verb = if on { "remove" } else { "add" };
                    run_in_background(vec!["tag".into(), verb.into(), name, "--".into(), file]);
                    Ok(())
                });
            }
            section.add_item("+ New list…", move |ctx| {
                ask(ctx, "New list", "Name:", vec!["tag", "add"], f2);
                Ok(())
            });
            Some(section)
        })
        .list_section(ctx, move |mut section| {
            for (sign, genre) in of.genres_added.iter().map(|g| ('+', g)).chain(of.genres_excluded.iter().map(|g| ('-', g))) {
                let (file, genre) = (f4.clone(), genre.clone());
                section.add_item(format!("Undo genre {sign}{genre}"), move |_| {
                    run_in_background(vec!["genre".into(), "reset".into(), genre, "--".into(), file]);
                    Ok(())
                });
            }
            let f5 = f3.clone();
            section.add_item("Add a genre…", move |ctx| {
                ask(ctx, "Add a genre to this song", "Genre:", vec!["genre", "add"], f3);
                Ok(())
            });
            section.add_item("Remove a MusicBrainz genre…", move |ctx| {
                ask(ctx, "Exclude a genre for this song", "Genre:", vec!["genre", "exclude"], f5);
                Ok(())
            });
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}
