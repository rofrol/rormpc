//! rormpc: Play's Browse (plans/combined-view.md "Play absorbs the browsing tabs", phase 3b): what `P` plays and
//! in which order, and which stored playlists are generated (read-only in the editor).
//!
//! - `P` on a container (an album, artist, folder or list, or several marked ones) plays its songs; on a song it
//!   plays the column the song is in (the album, folder or list) from that song. Order: an album in disc/track
//!   order, an artist by album date (`OriginalDate`, else `Date`) then disc/track, a folder in filename order, a list
//!   in its own order.
//! - Generated playlists ("Tag NAME", "Smart NAME", rormpc-tools' "Hits …" exports, the Live playlists' `.m3u`)
//!   are rewritten by their owner: the editor shows them, but does not move, remove or rename in them.

use anyhow::Result;
use rmpc_mpd::commands::Song;

use crate::{
    config::tabs::PlayGrouping,
    ctx::Ctx,
    ui::{browser::BrowserPane, dir_or_song::DirOrSong, rormpc_upnext::Collection, rormpc_years::shown_date},
};

/// Prefixes of the playlists rormpc-tools writes (its `hits.GENERATED_PLAYLISTS`, plus tags and smart lists).
const GENERATED: &[(&str, &str)] = &[
    ("Tag ", "a tag list: change it with the Tags… menu"),
    ("Smart ", "a smart list's export: change the list (L) instead"),
    ("Hits ", "written by rormpc-tools"),
    ("My charts ", "written by rormpc-tools"),
    ("Library ", "written by rormpc-tools"),
    ("Likes ", "written by rormpc-tools"),
    ("Recommendations ", "written by rormpc-tools"),
    ("My playlists ", "written by rormpc-tools"),
    ("LB ", "written by rormpc-tools"),
    ("Folder ", "written by rormpc-tools"),
    ("Skipped", "written by rormpc-tools"),
    ("Not finished", "written by rormpc-tools"),
];

/// Why the stored playlist `name` is read-only here, or None when it can be edited.
pub fn generated(name: &str, live: &[String]) -> Option<&'static str> {
    if live.iter().any(|l| l == name) {
        return Some("a Live playlist: accept or reject its items in the Live inbox (0)");
    }
    GENERATED.iter().find(|(prefix, _)| name.starts_with(prefix)).map(|(_, why)| *why)
}

/// The number at the start of a tag ("3/12" -> 3); songs without one sort first.
fn number(song: &Song, tag: &str) -> u32 {
    song.metadata.get(tag).map_or(0, |t| {
        t.first().chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)
    })
}

fn tag(song: &Song, tag: &str) -> String {
    song.metadata.get(tag).map(|t| t.first().to_owned()).unwrap_or_default()
}

/// Put one container's songs into the order its kind plays in.
pub fn sort_for(kind: &str, songs: &mut [Song]) {
    match kind {
        "album" => songs.sort_by_key(|s| (number(s, "disc"), number(s, "track"))),
        "artist" => songs.sort_by_cached_key(|s| (shown_date(s), tag(s, "album"), number(s, "disc"), number(s, "track"))),
        "directory" => songs.sort_by(|a, b| a.file.cmp(&b.file)),
        _ => {}
    }
}

/// The kind of a container in `grouping` at `depth` (0: the root list).
fn container_kind(grouping: PlayGrouping, depth: usize) -> &'static str {
    match (grouping, depth) {
        (PlayGrouping::Folders, _) => "directory",
        (PlayGrouping::Lists, _) => "playlist",
        (PlayGrouping::Albums, _) => "album",
        (PlayGrouping::Artists | PlayGrouping::AlbumArtists, 0) => "artist",
        (PlayGrouping::Artists | PlayGrouping::AlbumArtists, _) => "album",
    }
}

fn item_name(item: &DirOrSong) -> String {
    match item {
        DirOrSong::Dir { name, display_name, .. } => display_name.clone().unwrap_or_else(|| name.clone()),
        DirOrSong::Song(song) => song.file.rsplit('/').next().unwrap_or(&song.file).to_owned(),
    }
}

/// The name a container is remembered by in source.json and shown in the header.
fn container_name(grouping: PlayGrouping, path: &[String], depth: usize, own: String) -> String {
    match grouping {
        PlayGrouping::Folders => path.iter().take(depth).cloned().chain(std::iter::once(own)).collect::<Vec<_>>().join("/"),
        PlayGrouping::Artists | PlayGrouping::AlbumArtists if depth >= 1 => format!("{} / {own}", path[0]),
        _ => own,
    }
}

/// What `P` plays in `browser` (a Browse grouping), or None when nothing is selected. Lists the songs of the
/// selected containers through MPD (a short sync query, as the browsers' menu does).
pub(in crate::ui) fn collection<B: BrowserPane<DirOrSong>>(browser: &B, grouping: PlayGrouping, ctx: &Ctx) -> Result<Option<Collection>> {
    let stack = browser.stack();
    let path = stack.path().as_slice().to_vec();
    let depth = path.len();
    let current = stack.current();
    let hovered_song = match current.selected() {
        Some(DirOrSong::Song(song)) if current.marked().is_empty() => Some(song.file.clone()),
        _ => None,
    };
    if let Some(hovered) = hovered_song {
        // a song: its column (album, folder, list) from this song
        let kind = container_kind(grouping, depth.saturating_sub(1));
        let mut songs: Vec<Song> = current
            .items
            .iter()
            .filter_map(|i| match i {
                DirOrSong::Song(s) => Some(s.clone()),
                DirOrSong::Dir { .. } => None,
            })
            .collect();
        sort_for(kind, &mut songs);
        let own = stack.previous().and_then(|p| p.selected()).map_or_else(|| path.last().cloned().unwrap_or_default(), item_name);
        let name = container_name(grouping, &path, depth.saturating_sub(1), own);
        let start = songs.iter().position(|s| s.file == hovered).unwrap_or(0);
        return Ok(Some(Collection { kind: kind.to_owned(), name, files: songs.into_iter().map(|s| s.file).collect(), start }));
    }
    let items: Vec<DirOrSong> = browser.items(false).map(|(_, i)| i.clone()).collect();
    if items.is_empty() {
        return Ok(None);
    }
    let kind = container_kind(grouping, depth);
    let mut files = Vec::new();
    for item in &items {
        let list = browser.list_songs_in_item(item.clone());
        let mut songs = ctx.query_sync(list)?;
        if matches!(item, DirOrSong::Dir { .. }) {
            sort_for(kind, &mut songs);
        }
        files.extend(songs.into_iter().map(|s| s.file));
    }
    let (kind, name) = match items.as_slice() {
        [one @ DirOrSong::Dir { .. }] => (kind.to_owned(), container_name(grouping, &path, depth, item_name(one))),
        [DirOrSong::Song(_), ..] if items.iter().all(|i| matches!(i, DirOrSong::Song(_))) => {
            // marked songs of one container: that container, in the marked songs' order
            let own = stack.previous().and_then(|p| p.selected()).map_or_else(String::new, item_name);
            ("selection".to_owned(), format!("{} songs of {}", items.len(), container_name(grouping, &path, depth.saturating_sub(1), own)))
        }
        _ => ("selection".to_owned(), format!("{} {}s", items.len(), crate::ui::rormpc_upnext::kind_label(kind))),
    };
    Ok(Some(Collection { kind, name, files, start: 0 }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rmpc_mpd::commands::metadata_tag::MetadataTag;

    use super::*;

    fn song(file: &str, tags: &[(&str, &str)]) -> Song {
        Song {
            file: file.to_owned(),
            metadata: tags.iter().map(|(k, v)| ((*k).to_owned(), MetadataTag::Single((*v).to_owned()))).collect::<HashMap<_, _>>(),
            ..Default::default()
        }
    }

    fn files(songs: &[Song]) -> Vec<&str> {
        songs.iter().map(|s| s.file.as_str()).collect()
    }

    #[test]
    fn an_album_plays_in_disc_and_track_order_and_a_folder_by_filename() {
        let mut album = vec![
            song("b", &[("disc", "2"), ("track", "1")]),
            song("c", &[("disc", "1/2"), ("track", "10/12")]),
            song("a", &[("disc", "1"), ("track", "2")]),
        ];
        sort_for("album", &mut album);
        assert_eq!(files(&album), ["a", "c", "b"]);
        let mut folder = vec![song("x/10 b.mp3", &[("track", "1")]), song("x/02 a.mp3", &[("track", "9")])];
        sort_for("directory", &mut folder);
        assert_eq!(files(&folder), ["x/02 a.mp3", "x/10 b.mp3"]);
        // a list keeps its own order
        let mut list = vec![song("z", &[]), song("a", &[])];
        sort_for("playlist", &mut list);
        assert_eq!(files(&list), ["z", "a"]);
    }

    #[test]
    fn an_artist_plays_album_by_album_in_date_order() {
        let mut songs = vec![
            song("late-1", &[("date", "1990"), ("album", "L"), ("track", "1")]),
            song("early-2", &[("date", "1982"), ("album", "E"), ("track", "2")]),
            song("early-1", &[("date", "1982"), ("album", "E"), ("track", "1")]),
        ];
        sort_for("artist", &mut songs);
        assert_eq!(files(&songs), ["early-1", "early-2", "late-1"]);
        // the original release orders a reissue among the albums of its time
        let mut songs = vec![
            song("mid", &[("date", "1985"), ("album", "M"), ("track", "1")]),
            song("reissue", &[("date", "2002"), ("originaldate", "1979"), ("album", "R"), ("track", "1")]),
        ];
        sort_for("artist", &mut songs);
        assert_eq!(files(&songs), ["reissue", "mid"]);
    }

    #[test]
    fn generated_playlists_are_read_only_with_their_owner_named() {
        let live = vec!["Discover copy".to_owned()];
        assert!(generated("Tag God", &live).is_some_and(|w| w.contains("tag")));
        assert!(generated("Smart 80s party", &live).is_some_and(|w| w.contains("smart list")));
        assert!(generated("Hits 1980s top100", &live).is_some());
        assert!(generated("Not finished", &live).is_some());
        assert!(generated("Discover copy", &live).is_some_and(|w| w.contains("Live")));
        assert_eq!(generated("melancholic", &live), None);
        assert_eq!(generated("Tagged by hand", &live), None); // "Tag " needs the space
    }

    #[test]
    fn containers_are_named_by_grouping_and_depth() {
        let path = vec!["Toto".to_owned()];
        assert_eq!(container_kind(PlayGrouping::Artists, 0), "artist");
        assert_eq!(container_kind(PlayGrouping::Artists, 1), "album");
        assert_eq!(container_kind(PlayGrouping::Folders, 2), "directory");
        assert_eq!(container_name(PlayGrouping::Artists, &path, 1, "Toto IV".to_owned()), "Toto / Toto IV");
        let dirs = vec!["rock".to_owned(), "toto".to_owned()];
        assert_eq!(container_name(PlayGrouping::Folders, &dirs, 2, "IV".to_owned()), "rock/toto/IV");
        assert_eq!(container_name(PlayGrouping::Folders, &dirs, 1, "toto".to_owned()), "rock/toto");
        assert_eq!(container_name(PlayGrouping::Lists, &[], 0, "melancholic".to_owned()), "melancholic");
    }
}
