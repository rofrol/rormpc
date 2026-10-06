//! rormpc: a song's genres (`hits genres of`, rormpc-tools): "Pin genre…" in the Queue and Hits menus pins one
//! as a checkbox of the Hits filter column (`hits genres pin`, the same pins file the genre explorer writes), and
//! "Add to playlist…" offers them as names for a new playlist. Genres are the exact names `hits` uses (MusicBrainz
//! recording genres, else the artist's, plus `musicdb genre` corrections), so a pinned box matches what it says.

use std::{collections::HashSet, path::PathBuf, process::Command};

use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::macros::{modal, status_error, status_info},
    ui::modals::menu::modal::MenuModal,
};

const HITS: &str = "hits";

#[derive(Debug, Clone, Deserialize)]
pub struct SongGenres {
    pub genres: Vec<String>,
    /// "recording", "artist" or "unknown"
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct GenresOf {
    version: u32,
    songs: Vec<SongGenres>,
}

/// `hits genres of --json -- FILE...`: one entry per file MPD knows, in the given order.
pub fn genres_of(files: &[String]) -> Result<Vec<SongGenres>, String> {
    let out = Command::new(HITS)
        .args(["genres", "of", "--json", "--"])
        .args(files)
        .output()
        .map_err(|err| crate::shared::dependencies::cannot_run(HITS, &err))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("failed");
        return Err(format!("hits genres of: {last} (needs rormpc-tools 0.2.29 or later)"));
    }
    let parsed: GenresOf = serde_json::from_slice(&out.stdout).map_err(|e| format!("hits genres of: {e}"))?;
    if parsed.version != 1 {
        return Err(format!("hits genres of: unsupported version {}", parsed.version));
    }
    Ok(parsed.songs)
}

/// The pins file `hits genres pin` writes and the Hits pane reads.
pub fn pins_path() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config"))
        .join("rormpc-tools/hits-genres.json")
}

/// The pinned genres, None when the file is missing or unreadable (the Hits pane then uses its built-in list).
pub fn pins() -> Option<Vec<String>> {
    #[derive(Deserialize)]
    struct Pins {
        pins: Vec<String>,
    }
    std::fs::read_to_string(pins_path()).ok().and_then(|t| serde_json::from_str::<Pins>(&t).ok()).map(|p| p.pins)
}

/// Genres every one of the songs has, in the first song's order: names offered for a new playlist.
pub fn shared_genres(songs: &[SongGenres]) -> Vec<String> {
    let Some((first, rest)) = songs.split_first() else { return Vec::new() };
    first.genres.iter().filter(|g| rest.iter().all(|s| s.genres.contains(g))).cloned().collect()
}

/// "Pin genre…" for a library file.
pub fn open_pin_menu_for_file(ctx: &Ctx, file: String, what: String) {
    match genres_of(std::slice::from_ref(&file)) {
        Ok(songs) => match songs.into_iter().next() {
            Some(song) => open_pin_menu(ctx, what, song.genres, &song.source),
            None => status_error!("MPD doesn't know {file}"),
        },
        Err(err) => status_error!("{err}"),
    }
}

/// Pin or unpin one of `genres` (the song's, from `source`) as a Hits checkbox.
pub fn open_pin_menu(ctx: &Ctx, what: String, genres: Vec<String>, source: &str) {
    if genres.is_empty() {
        return status_info!("No genre known for {what} (Tags… can add one)");
    }
    let pinned: HashSet<String> = pins().unwrap_or_default().into_iter().collect();
    let from = match source {
        "recording" => "the recording's MusicBrainz genres",
        "artist" => "the artist's MusicBrainz genres",
        "chart" => "the chart row",
        _ => "MusicBrainz",
    };
    let menu = MenuModal::new(ctx)
        .width(60)
        .list_section(ctx, move |mut section| {
            section.add_item(format!("Hits checkboxes from {what} ({from}):"), |_| Ok(()));
            for genre in genres {
                let on = pinned.contains(&genre);
                let label = if on { format!("📌 {genre}  (unpin)") } else { format!("   {genre}  (pin)") };
                section.add_item(label, move |_| {
                    let verb = if on { "unpin" } else { "pin" };
                    std::thread::spawn(move || {
                        match Command::new(HITS).args(["genres", verb, &genre]).output() {
                            Ok(o) if o.status.success() => status_info!("{verb}ned {genre} in Hits"),
                            Ok(_) => status_error!("hits genres {verb} {genre} failed"),
                            Err(err) => status_error!("{}", crate::shared::dependencies::cannot_run(HITS, &err)),
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn song(genres: &[&str]) -> SongGenres {
        SongGenres { genres: genres.iter().map(|g| (*g).to_owned()).collect(), source: "recording".into() }
    }

    #[test]
    fn shared_genres_keeps_the_first_songs_order() {
        assert_eq!(shared_genres(&[song(&["synth-pop", "pop", "new wave"]), song(&["new wave", "synth-pop"])]), [
            "synth-pop",
            "new wave"
        ]);
        assert!(shared_genres(&[]).is_empty());
        assert!(shared_genres(&[song(&["rock"]), song(&[])]).is_empty());
    }

    #[test]
    fn genres_of_json_shape() {
        // the shape rormpc-tools' tests/test_genres_of.py pins down
        let text = r#"{"version": 1, "songs": [{"file": "a.mp3", "genres": ["hip hop"], "source": "artist", "pinned": ["hip hop"]}]}"#;
        let parsed: GenresOf = serde_json::from_str(text).unwrap();
        assert_eq!(parsed.songs[0].genres, ["hip hop"]);
        assert_eq!(parsed.songs[0].source, "artist");
    }
}
