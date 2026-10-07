//! rormpc: lyrics from `musicdb lyrics` (rormpc-tools, LRCLIB). Synced lyrics are ordinary `.lrc` files under
//! `lyrics_dir`, which upstream's Lyrics pane already reads. This adds what upstream lacks: plain lyrics (`.txt`
//! next to where the `.lrc` would be), scrolled along with the song; a note when there are none (instrumental,
//! nothing on LRCLIB, not checked yet) taken from `<lyrics_dir>/index.json`; "Choose lyrics…", a menu of the
//! LRCLIB entries for a song; and a Polish translation beside the original (`musicdb lyrics translate`, from
//! tekstowo.pl, stored in `<song stem>.pl.json` next to the lyrics).

use std::{collections::HashMap, process::Command};

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Text},
    widgets::{Paragraph, Wrap},
};
use serde::Deserialize;

use crate::{
    ctx::Ctx,
    shared::{
        events::AppEvent,
        macros::{modal, status_error, status_info},
    },
    ui::{UiEvent, modals::menu::modal::MenuModal},
};

const MUSICDB: &str = "musicdb";

/// What to show when the current song has no `.lrc`.
#[derive(Debug, Clone)]
pub enum Fallback {
    Plain(Vec<String>),
    Note(String),
}

#[derive(Debug, Deserialize)]
struct IndexEntry {
    state: String,
}

/// Plain lyrics or a note for the playing song; None without lyrics_dir or a song.
pub fn fallback(ctx: &Ctx) -> Option<Fallback> {
    let song = ctx.current_song()?;
    let dir = ctx.config.lyrics_dir.as_ref()?;
    let txt = crate::shared::lrc::get_lrc_path(dir, &song.file).ok()?.with_extension("txt");
    if let Ok(text) = std::fs::read_to_string(&txt) {
        return Some(Fallback::Plain(text.lines().map(str::to_owned).collect()));
    }
    let index: HashMap<String, IndexEntry> = std::fs::read_to_string(std::path::Path::new(dir).join("index.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let note = match index.get(&song.file).map(|e| e.state.as_str()) {
        Some("instrumental") => "Instrumental",
        Some("none") => "No lyrics on LRCLIB within 2 s of this file's length\n\nQueue menu: Choose lyrics…",
        Some("untagged") => "No artist/title tags to look the lyrics up by",
        Some(_) => return None, // synced: the .lrc is being (re)loaded
        None => "Lyrics not checked yet\n\nQueue menu: Choose lyrics…",
    };
    Some(Fallback::Note(note.to_owned()))
}

/// Plain lyrics scrolled in proportion to the song's progress (they have no timestamps); a note centred.
pub fn render(frame: &mut Frame, area: Rect, ctx: &Ctx, fallback: &Fallback) {
    match fallback {
        Fallback::Plain(lines) => {
            let text: Vec<Line> = lines.iter().map(|l| Line::from(l.clone())).collect();
            let wrapped = lines
                .iter()
                .map(|l| textwrap::wrap(l, usize::from(area.width).max(1)).len().max(1))
                .sum::<usize>();
            let hidden = wrapped.saturating_sub(usize::from(area.height));
            let total = ctx.status.duration.as_secs_f64();
            let progress = if total > 0.0 { (ctx.status.elapsed.as_secs_f64() / total).clamp(0.0, 1.0) } else { 0.0 };
            let scroll = (hidden as f64 * progress) as u16;
            let paragraph = Paragraph::new(Text::from(text))
                .alignment(Alignment::Center)
                .style(ctx.config.as_text_style())
                .wrap(Wrap { trim: false })
                .scroll((scroll, 0));
            frame.render_widget(paragraph, area);
        }
        Fallback::Note(note) => {
            let lines: Vec<Line> = note.lines().map(|l| Line::from(l.to_owned())).collect();
            let top = area.height.saturating_sub(lines.len() as u16) / 2;
            let area = Rect { y: area.y + top, height: area.height.saturating_sub(top), ..area };
            let paragraph = Paragraph::new(Text::from(lines))
                .alignment(Alignment::Center)
                .style(Style::default().add_modifier(Modifier::DIM))
                .wrap(Wrap { trim: false });
            frame.render_widget(paragraph, area);
        }
    }
}

#[derive(Debug, Deserialize)]
struct Candidates {
    candidates: Vec<Candidate>,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    id: u64,
    #[serde(default)]
    artist: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    album: Option<String>,
    delta: f64,
    synced: bool,
    plain: bool,
    instrumental: bool,
}

fn run(args: &[&str]) -> Result<String, String> {
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

/// Run `musicdb lyrics …` in the background, report its last line, then make the Lyrics pane reload.
fn run_then_reload(ctx: &Ctx, args: Vec<String>) {
    let sender = ctx.app_event_sender.clone();
    std::thread::spawn(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match run(&args) {
            Ok(out) => status_info!("{}", out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("lyrics updated")),
            Err(err) => status_error!("musicdb lyrics: {err}"),
        }
        let _ = sender.send(AppEvent::UiEvent(UiEvent::LyricsIndexed));
    });
}

/// "Choose lyrics…": the LRCLIB entries for a song, closest length first. Looking them up takes a moment
/// (one LRCLIB search), so the menu opens after it.
pub fn open_chooser(ctx: &Ctx, file: String) {
    let found = run(&["lyrics", "candidates", &file])
        .and_then(|out| serde_json::from_str::<Candidates>(&out).map_err(|e| e.to_string()));
    let candidates = match found {
        Ok(c) => c.candidates,
        Err(err) => return status_error!("musicdb lyrics: {err}"),
    };
    let auto = file.clone();
    let menu = MenuModal::new(ctx)
        .width(90) // "synced  -8 s  Artist - Title  (Album)" needs room
        .list_section(ctx, move |mut section| {
            if candidates.is_empty() {
                section.add_item("LRCLIB has nothing under this artist and title", |_| Ok(()));
            }
            for c in candidates.into_iter().take(15) {
                let kind = if c.synced { "synced" } else if c.instrumental { "instrumental" } else if c.plain { "plain" } else { "empty" };
                let label = format!(
                    "{kind:<12} {:>+4.0} s  {} - {}{}",
                    c.delta,
                    c.artist.unwrap_or_default(),
                    c.title.unwrap_or_default(),
                    c.album.map(|a| format!("  ({a})")).unwrap_or_default()
                );
                let (file, id) = (file.clone(), c.id.to_string());
                section.add_item(label, move |ctx| {
                    run_then_reload(ctx, vec!["lyrics".into(), "use".into(), file, id]);
                    Ok(())
                });
            }
            Some(section)
        })
        .list_section(ctx, move |mut section| {
            section.add_item("Fetch automatically (closest length)", move |ctx| {
                run_then_reload(ctx, vec!["lyrics".into(), "fetch".into(), auto]);
                Ok(())
            });
            Some(section)
        })
        .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
        .build();
    modal!(ctx, menu);
}

/// Below this width the Lyrics pane shows one column and h/l switches between the original and the translation.
pub const TWO_COLUMNS_MIN_WIDTH: u16 = 100;
const GAP: u16 = 3;

#[derive(Debug, Clone, Deserialize)]
pub struct Unit {
    pub ids: Vec<usize>,
    pub text: Vec<String>,
    /// one line of a stanza that pairs line by line (in a translation aligned by stanza)
    #[serde(default)]
    pub line: bool,
}

#[derive(Debug, Deserialize)]
struct Sidecar {
    state: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    original_hash: Option<String>,
    #[serde(default)]
    original_lang: Option<String>,
    #[serde(default)]
    pairing: Option<String>,
    #[serde(default)]
    units: Vec<Unit>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pairing {
    /// each unit is one original line and its translation: the current line is highlighted on both sides
    Line,
    /// a unit is a run of original lines and a stanza of the translation, or (`Unit::line`) one line of a stanza
    /// that pairs line by line
    Stanza,
    /// one unit, the whole text: the translation scrolls along on its own
    Whole,
}

/// The original's lines by id (as `musicdb lyrics translate` numbers them) and what the sidecar says about its
/// Polish translation.
#[derive(Debug)]
pub struct Translation {
    pub lines: Vec<String>,
    /// empty: nothing to show beside the original
    pub units: Vec<Unit>,
    pub pairing: Pairing,
    /// the status row under the lyrics
    pub status: String,
    /// what Confirm (Enter) does: look the translation up (None: nothing)
    pub action: Option<&'static str>,
}

/// What the Lyrics pane shows when it has a translation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Both,
    Original,
    Polish,
}

/// FNV-1a 64 of the original's lines joined by "\n": `musicdb lyrics translate` stores it to mark a translation
/// stale once the lyrics change.
pub fn lines_hash(lines: &[String]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in lines.join("\n").bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
    }
    format!("fnv1a64:{h:016x}")
}

/// The translation state of the playing song's lyrics (`lines`, numbered like the sidecar's ids).
pub fn translation(ctx: &Ctx, lines: Vec<String>, estimated: bool) -> Option<Translation> {
    let song = ctx.current_song()?;
    let dir = ctx.config.lyrics_dir.as_ref()?;
    let path = crate::shared::lrc::get_lrc_path(dir, &song.file).ok()?.with_extension("pl.json");
    let sidecar = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Sidecar>(&t).ok());
    Some(translation_from(lines, estimated, sidecar))
}

fn translation_from(lines: Vec<String>, estimated: bool, sidecar: Option<Sidecar>) -> Translation {
    let mut tr = Translation {
        lines,
        units: Vec::new(),
        pairing: Pairing::Whole,
        status: String::new(),
        action: Some("look it up on tekstowo.pl"),
    };
    let Some(s) = sidecar else {
        "No Polish translation".clone_into(&mut tr.status);
        return tr;
    };
    let mine = s.kind.as_deref() == Some("mine");
    if mine {
        tr.action = None;
    }
    if s.state == "original_pl" || s.original_lang.as_deref() == Some("pl") {
        "Polish original".clone_into(&mut tr.status);
        tr.action = None;
        return tr;
    }
    if s.original_hash.as_ref().is_some_and(|h| *h != lines_hash(&tr.lines)) {
        tr.status = if mine {
            "The lyrics changed since your translation: not shown".to_owned()
        } else {
            tr.action = Some("look it up again");
            "The lyrics changed since the translation".to_owned()
        };
        return tr;
    }
    tr.status = match s.state.as_str() {
        "translated" => {
            tr.pairing = match s.pairing.as_deref() {
                Some("line") => Pairing::Line,
                Some("stanza") => Pairing::Stanza,
                _ => Pairing::Whole,
            };
            let n = tr.lines.len();
            tr.units = s.units.into_iter().filter(|u| !u.ids.is_empty() && u.ids.iter().all(|&i| i < n)).collect();
            let kind = match s.kind.as_deref() {
                Some("machine") => "machine translation (tekstowo.pl AI)",
                Some("mine") => "your translation",
                _ => "translation from tekstowo.pl",
            };
            let how = match tr.pairing {
                Pairing::Line => "",
                Pairing::Stanza => " · aligned by stanza",
                Pairing::Whole => " · not aligned",
            };
            let est = if estimated && tr.pairing != Pairing::Whole { " · current line estimated" } else { "" };
            if !mine {
                tr.action = Some("look it up again");
            }
            format!("Polish: {kind}{how}{est}")
        }
        "none" => {
            tr.action = Some("check again");
            "tekstowo.pl has no Polish translation".to_owned()
        }
        "not_found" => {
            tr.action = Some("search again");
            "Not on tekstowo.pl".to_owned()
        }
        "mismatch" => {
            tr.action = Some("search again");
            "tekstowo.pl only has other lyrics under this title".to_owned()
        }
        "instrumental" => {
            tr.action = None;
            String::new()
        }
        other => format!("Translation: {other}"),
    };
    tr
}

/// The original line id the song is at for plain lyrics: its progress spread over the non-empty lines.
pub fn estimated_line(lines: &[String], ctx: &Ctx) -> Option<usize> {
    let total = ctx.status.duration.as_secs_f64();
    if total <= 0.0 {
        return None;
    }
    let progress = (ctx.status.elapsed.as_secs_f64() / total).clamp(0.0, 1.0);
    let sung: Vec<usize> = (0..lines.len()).filter(|&i| !lines[i].trim().is_empty()).collect();
    let k = ((sung.len() as f64 * progress) as usize).min(sung.len().checked_sub(1)?);
    Some(sung[k])
}

/// One screen row of the two-column layout.
#[derive(Debug, Default, PartialEq)]
struct Row {
    left: String,
    /// the original line this row shows (highlighted when current)
    left_id: Option<usize>,
    right: String,
    /// the original line the translation row stands for, only when the pairing is 1:1
    right_id: Option<usize>,
    /// the original ids of the block this row belongs to
    block: (usize, usize),
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let lines: Vec<String> = textwrap::wrap(text, width).into_iter().map(|l| l.into_owned()).collect();
    if lines.is_empty() { vec![String::new()] } else { lines }
}

/// Blocks of original lines beside their translation, wrapped to the column widths (0: leave that side out).
fn rows(tr: &Translation, lw: usize, rw: usize) -> Vec<Row> {
    let mut unit_of = HashMap::new();
    for (u, unit) in tr.units.iter().enumerate() {
        for &id in &unit.ids {
            unit_of.insert(id, u);
        }
    }
    let mut out = Vec::new();
    let mut id = 0;
    while id < tr.lines.len() {
        let unit = unit_of.get(&id).map(|&u| &tr.units[u]).filter(|u| u.ids.iter().min() == Some(&id));
        let (last, right, line): (usize, &[String], bool) = match unit {
            Some(u) => (u.ids.iter().copied().max().unwrap_or(id), &u.text, u.line),
            None => (id, &[], false),
        };
        let left: Vec<(String, usize)> =
            (id..=last).flat_map(|i| wrap(&tr.lines[i], lw).into_iter().map(move |l| (l, i))).collect();
        let one_to_one = (tr.pairing == Pairing::Line || line) && id == last && right.len() == 1;
        let right: Vec<String> = right.iter().flat_map(|l| wrap(l, rw)).collect();
        // translation only: a blank original line still separates the stanzas
        let blank = lw == 0 && unit.is_none() && tr.lines[id].trim().is_empty();
        for k in 0..left.len().max(right.len()).max(usize::from(blank)) {
            let (l, lid) = left.get(k).map_or((String::new(), None), |(l, i)| (l.clone(), Some(*i)));
            out.push(Row {
                left: l,
                left_id: lid,
                right: right.get(k).cloned().unwrap_or_default(),
                right_id: (one_to_one && k < right.len()).then_some(id),
                block: (id, last),
            });
        }
        id = last + 1;
    }
    out
}

/// The original beside its translation (or one of them, `view`), the current line (`current`, an original id)
/// in the middle and highlighted; `reached`: the song is past the first line.
pub fn render_translation(
    frame: &mut Frame,
    area: Rect,
    ctx: &Ctx,
    tr: &Translation,
    current: Option<usize>,
    reached: bool,
    view: View,
) {
    let default_style = ctx.config.as_text_style();
    let highlight = if reached { default_style.patch(ctx.config.theme.highlighted_item_style) } else { default_style };
    let (lw, rw) = match view {
        View::Both => {
            let lw = area.width.saturating_sub(GAP) / 2;
            (lw, area.width.saturating_sub(GAP + lw))
        }
        View::Original => (area.width, 0),
        View::Polish => (0, area.width),
    };
    // a translation that is not aligned scrolls on its own, in proportion to the original's current line
    let whole = tr.pairing == Pairing::Whole;
    let all = if whole && rw > 0 {
        let text: Vec<Line> = tr.units.iter().flat_map(|u| u.text.iter()).map(|l| Line::from(l.clone())).collect();
        let wrapped = tr.units.iter().flat_map(|u| u.text.iter()).map(|l| wrap(l, usize::from(rw)).len()).sum::<usize>();
        let hidden = wrapped.saturating_sub(usize::from(area.height));
        let progress = current.map_or(0.0, |c| c as f64 / tr.lines.len().max(1) as f64);
        let x = area.x + lw + if lw > 0 { GAP } else { 0 };
        let right = Rect { x, width: rw, ..area };
        let paragraph = Paragraph::new(Text::from(text))
            .style(default_style)
            .wrap(Wrap { trim: false })
            .scroll(((hidden as f64 * progress) as u16, 0));
        frame.render_widget(paragraph, right);
        if lw == 0 {
            return;
        }
        rows(tr, usize::from(lw), 0)
    } else {
        rows(tr, usize::from(lw), usize::from(rw))
    };
    // the current line's row; a line the translation leaves out (translation only): the last row before it
    let at = current
        .and_then(|c| {
            all.iter()
                .position(|r| r.left_id == Some(c))
                .or_else(|| all.iter().position(|r| r.block.0 <= c && c <= r.block.1))
                .or_else(|| all.iter().rposition(|r| r.block.1 < c))
        })
        .unwrap_or(0);
    let middle = usize::from(area.height.saturating_sub(1) / 2);
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for y in 0..usize::from(area.height) {
        let row = (at + y).checked_sub(middle).and_then(|i| all.get(i));
        let Some(row) = row else {
            left.push(Line::default());
            right.push(Line::default());
            continue;
        };
        let style = |id: Option<usize>| if id.is_some() && id == current { highlight } else { default_style };
        left.push(Line::styled(row.left.clone(), style(row.left_id)));
        right.push(Line::styled(row.right.clone(), style(row.right_id)));
    }
    if lw > 0 {
        frame.render_widget(Paragraph::new(Text::from(left)), Rect { width: lw, ..area });
    }
    if rw > 0 && !whole {
        let x = area.x + lw + if lw > 0 { GAP } else { 0 };
        frame.render_widget(Paragraph::new(Text::from(right)), Rect { x, width: rw, ..area });
    }
}

/// The status row under the lyrics.
pub fn render_status(frame: &mut Frame, area: Rect, status: &str) {
    let paragraph = Paragraph::new(Line::from(status.to_owned()))
        .alignment(Alignment::Center)
        .style(Style::default().add_modifier(Modifier::DIM))
        .wrap(Wrap { trim: true });
    frame.render_widget(paragraph, area);
}

/// Enter in the Lyrics pane: look the playing song's translation up in the background.
pub fn fetch_translation(ctx: &Ctx) {
    let Some(song) = ctx.current_song() else { return };
    status_info!("Looking up the Polish translation on tekstowo.pl…");
    run_then_reload(ctx, vec!["lyrics".into(), "translate".into(), song.file.clone()]);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    fn sidecar(lines: &[String], pairing: &str, units: Vec<Unit>) -> Sidecar {
        Sidecar {
            state: "translated".into(),
            kind: Some("human".into()),
            original_hash: Some(lines_hash(lines)),
            original_lang: Some("en".into()),
            pairing: Some(pairing.into()),
            units,
        }
    }

    fn unit(ids: &[usize], text: &[&str]) -> Unit {
        Unit { ids: ids.to_vec(), text: lines(text), line: false }
    }

    #[test]
    fn hash_matches_musicdb() {
        // the values tests/test_translation.py checks on the Python side
        assert_eq!(lines_hash(&[]), "fnv1a64:cbf29ce484222325");
        assert_eq!(lines_hash(&lines(&["a"])), "fnv1a64:af63dc4c8601ec8c");
    }

    #[test]
    fn line_pairs_highlight_both_sides() {
        let orig = lines(&["one", "two", "", "three"]);
        let s = sidecar(&orig, "line", vec![unit(&[0], &["jeden"]), unit(&[1], &["dwa"]), unit(&[3], &["trzy"])]);
        let tr = translation_from(orig, false, Some(s));
        assert_eq!(tr.status, "Polish: translation from tekstowo.pl");
        let r = rows(&tr, 20, 20);
        assert_eq!(r.len(), 4);
        assert_eq!((r[1].left.as_str(), r[1].right.as_str(), r[1].right_id), ("two", "dwa", Some(1)));
        assert_eq!((r[2].left_id, r[2].right_id), (Some(2), None));
    }

    #[test]
    fn stanzas_have_no_line_highlight_on_the_right() {
        let orig = lines(&["one", "two", "", "three"]);
        let s = sidecar(&orig, "stanza", vec![unit(&[0, 1], &["jeden i dwa"]), unit(&[3], &["trzy", "cztery"])]);
        let tr = translation_from(orig, true, Some(s));
        assert!(tr.status.ends_with("aligned by stanza · current line estimated"));
        let r = rows(&tr, 20, 20);
        assert_eq!(r.iter().map(|r| r.right.as_str()).collect::<Vec<_>>(), ["jeden i dwa", "", "", "trzy", "cztery"]);
        assert!(r.iter().all(|r| r.right_id.is_none()));
        assert_eq!(r[4].block, (3, 3));
        // a stanza that pairs line by line keeps its highlight
        let orig = lines(&["one", "two", "", "three"]);
        let paired = Unit { line: true, ..unit(&[3], &["trzy"]) };
        let s = sidecar(&orig, "stanza", vec![unit(&[0, 1], &["jeden i dwa"]), paired]);
        let r = rows(&translation_from(orig, false, Some(s)), 20, 20);
        assert_eq!((r[3].right.as_str(), r[3].right_id), ("trzy", Some(3)));
    }

    #[test]
    fn wrapped_lines_keep_blocks_aligned() {
        let orig = lines(&["a long original line here", "next"]);
        let s = sidecar(&orig, "line", vec![unit(&[0], &["krótko"]), unit(&[1], &["dalej"])]);
        let r = rows(&translation_from(orig, false, Some(s)), 10, 10);
        assert_eq!(r.iter().map(|r| r.right.as_str()).collect::<Vec<_>>(), ["krótko", "", "", "dalej"]);
        // translation only (narrow terminal): the original side is left out
        let orig = lines(&["one", "two"]);
        let s = sidecar(&orig, "line", vec![unit(&[0], &["jeden"]), unit(&[1], &["dwa"])]);
        let r = rows(&translation_from(orig, false, Some(s)), 0, 10);
        assert_eq!(r.iter().map(|r| r.right.as_str()).collect::<Vec<_>>(), ["jeden", "dwa"]);
        // ...where blank lines still separate stanzas and untranslated lines leave no row
        let orig = lines(&["one", "", "oh oh", "two"]);
        let s = sidecar(&orig, "line", vec![unit(&[0], &["jeden"]), unit(&[3], &["dwa"])]);
        let r = rows(&translation_from(orig, false, Some(s)), 0, 10);
        assert_eq!(r.iter().map(|r| r.right.as_str()).collect::<Vec<_>>(), ["jeden", "", "dwa"]);
    }

    #[test]
    fn stale_polish_and_missing_translations_show_one_column() {
        let orig = lines(&["one"]);
        let stale = sidecar(&lines(&["changed"]), "line", vec![unit(&[0], &["jeden"])]);
        let tr = translation_from(orig.clone(), false, Some(stale));
        assert!(tr.units.is_empty() && tr.action.is_some() && tr.status.starts_with("The lyrics changed"));
        let mut polish = sidecar(&orig, "line", Vec::new());
        polish.state = "original_pl".into();
        let tr = translation_from(orig.clone(), false, Some(polish));
        assert!(tr.units.is_empty() && tr.action.is_none() && tr.status == "Polish original");
        let tr = translation_from(orig.clone(), false, None);
        assert_eq!((tr.status.as_str(), tr.action), ("No Polish translation", Some("look it up on tekstowo.pl")));
        // ids out of range (a sidecar written for other lyrics) are dropped
        let s = sidecar(&orig, "line", vec![unit(&[5], &["pięć"])]);
        assert!(translation_from(orig, false, Some(s)).units.is_empty());
    }
}
