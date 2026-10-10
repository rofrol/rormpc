//! rormpc: the Hits selection rules (plans/combined-view.md, phase 1): ± set chips, Rank by and Years of, and
//! the printed formula. The selection itself stays in `hits` (rormpc-tools `hits_rules.py`):
//!
//!     (union of + sets, or the whole library when no set is +) − (union of − sets)
//!       ∩ period ∩ genres ∩ artists ∩ Top % ∩ owned
//!
//! Top % is cut in the rank's own population, so a song's rank never depends on which chips are on; with Rank by
//! none there is no Top %.
//!
//! Named sets (phase 5) are added through "+ set…" (`rormpc_sets`): a tag list `tag:NAME`, a stored MPD playlist
//! `playlist:NAME`, a Live playlist `live:ID` and a smart list `list:ID`, tri-state rows under the fixed ones.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

/// The fixed set chips: `hits --set` key, row label, name in the formula (as `hits_rules.SET_KINDS`).
pub const SETS: [(&str, &str, &str); 4] = [
    ("billboard", "Billboard US", "Billboard"),
    ("likes", "my likes", "Likes"),
    ("playlists", "my playlists", "Playlists"),
    ("recommended", "recommended", "Recommended"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankBy {
    /// best year-end position in the chosen years
    Billboard,
    /// my plays
    Plays,
    /// often played, not lately
    Rediscover,
    /// no ranking, so no Top %
    None,
}

impl RankBy {
    const ALL: [RankBy; 4] = [RankBy::Billboard, RankBy::Plays, RankBy::Rediscover, RankBy::None];

    pub fn arg(self) -> &'static str {
        match self {
            RankBy::Billboard => "billboard",
            RankBy::Plays => "plays",
            RankBy::Rediscover => "rediscover",
            RankBy::None => "none",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RankBy::Billboard => "Billboard",
            RankBy::Plays => "my plays",
            RankBy::Rediscover => "rediscover",
            RankBy::None => "none",
        }
    }

    /// `hits --rank`, the old "chart" and "listens" included.
    pub fn parse(s: &str) -> Option<RankBy> {
        match s {
            "billboard" | "chart" | "listens" => Some(RankBy::Billboard),
            "plays" => Some(RankBy::Plays),
            "rediscover" => Some(RankBy::Rediscover),
            "none" => Some(RankBy::None),
            _ => None,
        }
    }

    /// Years of follows Rank by unless chosen: Billboard -> chart year, my plays -> listened year, else release.
    pub fn default_years(self) -> YearsOf {
        match self {
            RankBy::Billboard => YearsOf::Chart,
            RankBy::Plays => YearsOf::Listened,
            RankBy::Rediscover | RankBy::None => YearsOf::Release,
        }
    }

    pub fn next(self, delta: i32) -> RankBy {
        cycle(&Self::ALL, self, delta)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YearsOf {
    Release,
    Chart,
    Listened,
}

impl YearsOf {
    pub fn arg(self) -> &'static str {
        match self {
            YearsOf::Release => "release",
            YearsOf::Chart => "chart",
            YearsOf::Listened => "listened",
        }
    }

    pub fn parse(s: &str) -> Option<YearsOf> {
        match s {
            "release" => Some(YearsOf::Release),
            "chart" => Some(YearsOf::Chart),
            "listened" => Some(YearsOf::Listened),
            _ => None,
        }
    }

    /// The Years of row's cycle: None (follows Rank by), then each axis.
    pub fn next(current: Option<YearsOf>, delta: i32) -> Option<YearsOf> {
        const ALL: [Option<YearsOf>; 4] = [None, Some(YearsOf::Release), Some(YearsOf::Chart), Some(YearsOf::Listened)];
        cycle(&ALL, current, delta)
    }
}

fn cycle<T: Copy + PartialEq>(all: &[T], current: T, delta: i32) -> T {
    let i = all.iter().position(|x| *x == current).unwrap_or(0);
    // one step back or forth (h/l, Space): the sign of delta is all that counts
    let step = usize::from(delta < 0) * (all.len() - 1) + usize::from(delta > 0);
    all[(i + step) % all.len()]
}

/// Off -> + -> − -> off, like a genre row.
pub fn next_sign(sign: i8) -> i8 {
    match sign {
        0 => 1,
        1 => -1,
        _ => 0,
    }
}

/// The old `--source` (+ `--sort`) of a result file written before the set chips: (sets, rank, years of).
pub fn from_source(source: Option<&str>, sort: Option<&str>) -> ([i8; 4], RankBy, YearsOf) {
    let by_sort = if sort == Some("rediscover") { RankBy::Rediscover } else { RankBy::Plays };
    match source {
        Some("likes") => ([0, 1, 0, 0], by_sort, YearsOf::Release),
        Some("library") => ([0; 4], by_sort, YearsOf::Release),
        Some("playlists") => ([0, 0, 1, 0], by_sort, YearsOf::Release),
        Some("mine") => ([0; 4], RankBy::Plays, YearsOf::Listened),
        Some("recs") => ([0, 0, 0, 1], RankBy::None, YearsOf::Release),
        _ => ([1, 0, 0, 0], RankBy::Billboard, YearsOf::Chart),
    }
}

/// A set added through "+ set…": its canonical `hits` key, its name in the rows and the formula ("Tag God"), and
/// its sign (-1 exclude, 0 off, 1 include; an added set keeps its row while off).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSet {
    pub key: String,
    pub name: String,
    pub sign: i8,
}

/// A named set's name from its key alone, as `hits_rules.set_name` makes it: "tag:God" -> "Tag God".
pub fn default_label(key: &str) -> String {
    let (kind, name) = key.split_once(':').unwrap_or((key, ""));
    let kind = match kind {
        "tag" => "Tag",
        "playlist" => "Playlist",
        "live" => "Live",
        "list" => "Smart",
        other => other,
    };
    format!("{kind} {name}")
}

fn names() -> &'static Mutex<HashMap<String, String>> {
    static NAMES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    NAMES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Keep a named set's name for scope labels elsewhere (a Live playlist and a smart list are keyed by id).
pub fn remember(key: &str, name: &str) {
    if let Ok(mut n) = names().lock() {
        n.insert(key.to_owned(), name.to_owned());
    }
}

/// A set's name: a fixed chip's label, else the name last seen for it, else a smart list's by the loaded lists,
/// else one made from its key.
pub fn set_label(key: &str) -> String {
    if let Some((_, label, _)) = SETS.iter().find(|(k, _, _)| *k == key) {
        return (*label).to_owned();
    }
    if let Some(name) = names().lock().ok().and_then(|n| n.get(key).cloned()) {
        return name;
    }
    match key.strip_prefix("list:") {
        Some(id) => format!("Smart {}", crate::ui::rormpc_smartlists::name_of(id)),
        None => default_label(key),
    }
}

/// The named sets of `--set` values (`"+tag:God"`), named by a result's `set_names` (else by their keys).
pub fn parse_named(values: &[String], set_names: Option<&HashMap<String, String>>) -> Vec<NamedSet> {
    let mut out: Vec<NamedSet> = Vec::new();
    for v in values {
        let (sign, key) = match v.strip_prefix('-') {
            Some(key) => (-1, key),
            None => (1, v.strip_prefix('+').unwrap_or(v)),
        };
        if !key.contains(':') {
            continue;
        }
        let name = set_names.and_then(|n| n.get(key).cloned()).unwrap_or_else(|| set_label(key));
        remember(key, &name);
        match out.iter_mut().find(|n| n.key == key) {
            Some(n) => n.sign = sign,
            None => out.push(NamedSet { key: key.to_owned(), name, sign }),
        }
    }
    out
}

/// The keys of the `+` sets, fixed ones first: the set scopes `+` / `-` on a row offer.
pub fn plus_keys(sets: [i8; 4], named: &[NamedSet]) -> Vec<String> {
    SETS.iter()
        .zip(sets)
        .filter(|(_, s)| *s > 0)
        .map(|((key, _, _), _)| (*key).to_owned())
        .chain(named.iter().filter(|n| n.sign > 0).map(|n| n.key.clone()))
        .collect()
}

/// `--set` values (`["+billboard", "-likes"]`) as chip signs; named sets (`parse_named`) are skipped.
pub fn parse_sets(values: &[String]) -> [i8; 4] {
    let mut sets = [0; 4];
    for v in values {
        let (sign, key) = match v.strip_prefix('-') {
            Some(key) => (-1, key),
            None => (1, v.strip_prefix('+').unwrap_or(v)),
        };
        if let Some(i) = SETS.iter().position(|(k, _, _)| *k == key) {
            sets[i] = sign;
        }
    }
    sets
}

fn union(names: &[&str]) -> String {
    if names.len() == 1 { names[0].to_owned() } else { format!("({})", names.join(" ∪ ")) }
}

/// " ∩ a" / " ∩ (a ∪ b)" for the included, " − x" per excluded (as `hits_rules._signed`).
fn signed(items: &[(String, i8)]) -> String {
    let inc: Vec<&str> = items.iter().filter(|(_, s)| *s > 0).map(|(n, _)| n.as_str()).collect();
    let mut out = if inc.is_empty() { String::new() } else { format!(" ∩ {}", union(&inc)) };
    for (name, _) in items.iter().filter(|(_, s)| *s < 0) {
        out = format!("{out} − {name}");
    }
    out
}

/// What `formula` needs from the filter column.
pub struct Rules<'a> {
    pub sets: [i8; 4],
    /// the sets added through "+ set…", after the fixed ones (as `rule_args` passes them)
    pub named: &'a [NamedSet],
    /// the `--years` passed, if any
    pub period: Option<String>,
    /// the Top % ranges passed ("1-10,11-20"), if any
    pub top: Option<String>,
    pub genres: &'a [(String, i8)],
    pub artists: &'a [(String, i8)],
    pub owned: bool,
}

/// The selection in one line, the same text `hits` prints and writes as "formula" (`hits_rules.formula`):
/// "(Billboard ∪ Tag God) − Recommended ∩ 1980-1989 ∩ Top 1-10% ∩ rock − country".
pub fn formula(r: &Rules) -> String {
    let names = |sign: i8| -> Vec<&str> {
        SETS.iter()
            .zip(r.sets)
            .filter(|(_, s)| *s == sign)
            .map(|((_, _, name), _)| *name)
            .chain(r.named.iter().filter(|n| n.sign == sign).map(|n| n.name.as_str()))
            .collect()
    };
    let (plus, minus) = (names(1), names(-1));
    let mut out = if plus.is_empty() { "Library".to_owned() } else { union(&plus) };
    if !minus.is_empty() {
        out = format!("{out} − {}", union(&minus));
    }
    if let Some(period) = &r.period {
        out = format!("{out} ∩ {period}");
    }
    if let Some(top) = &r.top {
        out = format!("{out} ∩ Top {top}%");
    }
    out.push_str(&signed(r.genres));
    out.push_str(&signed(r.artists));
    if r.owned {
        out.push_str(" ∩ owned");
    }
    out
}

/// 8312 -> "8,312", as Python's `{:,}`.
pub fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "… · 1,204 of 8,312", then " · +2 pinned" (rows a pin added, not in the first number) and " · 1 excluded" (rows
/// an exclusion took out): the text `hits` writes as "summary" (`hits_rules.summary`).
pub fn summary(text: &str, selected: u32, candidates: u32, pinned: u32, excluded: u32) -> String {
    let mut out = format!("{text} · {} of {}", thousands(selected.saturating_sub(pinned)), thousands(candidates));
    if pinned > 0 {
        out = format!("{out} · +{pinned} pinned");
    }
    if excluded > 0 {
        out = format!("{out} · {excluded} excluded");
    }
    out
}

/// Greedy word wrap for the filter column's summary (a long formula takes a few lines).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in text.split(' ') {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formula_matches_the_python_side() {
        // the same case as rormpc-tools' tests/test_hits_rules.py test_formula_reads_the_rules_in_order
        let genres = [("rock".to_owned(), 1), ("country".to_owned(), -1), ("pop".to_owned(), 0)];
        let artists = [("Queen".to_owned(), 1), ("Toto".to_owned(), 1)];
        let r = Rules {
            sets: [1, 1, 0, -1],
            named: &[],
            period: Some("1980-1989".to_owned()),
            top: Some("1-10".to_owned()),
            genres: &genres,
            artists: &artists,
            owned: true,
        };
        assert_eq!(
            formula(&r),
            "(Billboard ∪ Likes) − Recommended ∩ 1980-1989 ∩ Top 1-10% ∩ rock − country ∩ (Queen ∪ Toto) ∩ owned"
        );
        let r = Rules { sets: [0, -1, -1, 0], named: &[], period: None, top: None, genres: &[], artists: &[], owned: false };
        assert_eq!(formula(&r), "Library − (Likes ∪ Playlists)");
    }

    #[test]
    fn named_sets_follow_the_fixed_ones_in_the_formula() {
        // rormpc-tools' tests/test_hits_rules.py test_the_formula_names_named_sets
        let names = HashMap::from([("list:L1".to_owned(), "Smart 80s".to_owned())]);
        let values: Vec<String> =
            ["+billboard", "+tag:God", "-list:L1", "-playlist:Road trip"].iter().map(|v| (*v).to_owned()).collect();
        let named = parse_named(&values, Some(&names));
        assert_eq!(named.iter().map(|n| (n.name.as_str(), n.sign)).collect::<Vec<_>>(), [
            ("Tag God", 1),
            ("Smart 80s", -1),
            ("Playlist Road trip", -1)
        ]);
        let r = Rules { sets: parse_sets(&values), named: &named, period: None, top: None, genres: &[], artists: &[], owned: false };
        assert_eq!(formula(&r), "(Billboard ∪ Tag God) − (Smart 80s ∪ Playlist Road trip)");
        assert_eq!(plus_keys(r.sets, &named), ["billboard", "tag:God"]);
        assert_eq!(set_label("list:L1"), "Smart 80s"); // remembered for scope labels
        assert_eq!((set_label("likes"), default_label("live:yt-PL1")), ("my likes".to_owned(), "Live yt-PL1".to_owned()));
    }

    #[test]
    fn old_sources_map_onto_sets_rank_and_years() {
        assert_eq!(from_source(None, None), ([1, 0, 0, 0], RankBy::Billboard, YearsOf::Chart));
        assert_eq!(from_source(Some("likes"), Some("rediscover")), ([0, 1, 0, 0], RankBy::Rediscover, YearsOf::Release));
        assert_eq!(from_source(Some("library"), Some("plays")), ([0; 4], RankBy::Plays, YearsOf::Release));
        assert_eq!(from_source(Some("playlists"), None), ([0, 0, 1, 0], RankBy::Plays, YearsOf::Release));
        assert_eq!(from_source(Some("mine"), None), ([0; 4], RankBy::Plays, YearsOf::Listened));
        assert_eq!(from_source(Some("recs"), None), ([0, 0, 0, 1], RankBy::None, YearsOf::Release));
    }

    #[test]
    fn sets_parse_and_cycle() {
        let values = ["+billboard".to_owned(), "-likes".to_owned(), "+tag:God".to_owned(), "recommended".to_owned()];
        assert_eq!(parse_sets(&values), [1, -1, 0, 1]);
        assert_eq!((next_sign(0), next_sign(1), next_sign(-1)), (1, -1, 0));
        assert_eq!(RankBy::None.next(1), RankBy::Billboard);
        assert_eq!(YearsOf::next(None, 1), Some(YearsOf::Release));
        assert_eq!(YearsOf::next(Some(YearsOf::Listened), 1), None);
        assert_eq!(RankBy::parse("chart"), Some(RankBy::Billboard));
    }

    #[test]
    fn summary_matches_the_python_side() {
        // rormpc-tools' tests/test_hits_exceptions.py test_hits_json_reports_pins_and_exclusions
        assert_eq!(summary("Library − Likes ∩ 1980-1989", 2, 2, 1, 1), "Library − Likes ∩ 1980-1989 · 1 of 2 · +1 pinned · 1 excluded");
        assert_eq!(summary("Library", 1204, 8312, 0, 0), "Library · 1,204 of 8,312");
    }

    #[test]
    fn thousands_and_wrap() {
        assert_eq!((thousands(7), thousands(1204), thousands(8312), thousands(1_000_000)), (
            "7".to_owned(),
            "1,204".to_owned(),
            "8,312".to_owned(),
            "1,000,000".to_owned()
        ));
        assert_eq!(wrap("(Billboard ∪ Likes) − Recommended · 1,204 of 8,312", 22), [
            "(Billboard ∪ Likes) −",
            "Recommended · 1,204 of",
            "8,312"
        ]);
    }
}
