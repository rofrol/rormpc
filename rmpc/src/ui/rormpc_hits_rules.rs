//! rormpc: the Hits selection rules (plans/combined-view.md, phase 1): ± set chips, Rank by and Years of, and
//! the printed formula. The selection itself stays in `hits` (rormpc-tools `hits_rules.py`):
//!
//!     (union of + sets, or the whole library when no set is +) − (union of − sets)
//!       ∩ period ∩ genres ∩ artists ∩ Top % ∩ owned
//!
//! Top % is cut in the rank's own population, so a song's rank never depends on which chips are on; with Rank by
//! none there is no Top %.

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

/// `--set` values (`["+billboard", "-likes"]`) as chip signs; unknown kinds (later "+ set…" ones) are skipped.
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
    /// the `--years` passed, if any
    pub period: Option<String>,
    /// the Top % ranges passed ("1-10,11-20"), if any
    pub top: Option<String>,
    pub genres: &'a [(String, i8)],
    pub artists: &'a [(String, i8)],
    pub owned: bool,
}

/// The selection in one line, the same text `hits` prints and writes as "formula" (`hits_rules.formula`):
/// "(Billboard ∪ Likes) − Recommended ∩ 1980-1989 ∩ Top 1-10% ∩ rock − country".
pub fn formula(r: &Rules) -> String {
    let names = |sign: i8| -> Vec<&str> {
        SETS.iter().zip(r.sets).filter(|(_, s)| *s == sign).map(|((_, _, name), _)| *name).collect()
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
        let r = Rules { sets: [0, -1, -1, 0], period: None, top: None, genres: &[], artists: &[], owned: false };
        assert_eq!(formula(&r), "Library − (Likes ∪ Playlists)");
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
