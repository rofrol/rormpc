//! rormpc: live inline filtering, shared by the Queue and the Versions pane. Every typed word must appear in the
//! row, in any order (fzf's --exact; whole-row fuzzy matching let "lodz" match l…o…d…z scattered over a long
//! title, too loose for a filter that keeps the list's order). Matching is diacritic-folded, so "zolw" finds
//! "żółw" and "lodz" finds "Łódź"; nucleo matches the needle as given, so both sides are folded here. When no row
//! matches exactly, `find` returns close matches: each typed word within a few Damerau-Levenshtein edits of a word
//! of the row (never whole-row fuzzy), shown as "Close matches".

use std::collections::HashMap;

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    chars,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

use crate::config::keys::{Key, key::KeySequence};

/// Lowercase with diacritics removed. nucleo's table folds most Latin letters (ż, ó, é…); letters that Unicode
/// does not decompose are mapped here.
pub fn fold(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars().flat_map(char::to_lowercase) {
        match c {
            'ł' => out.push('l'),
            'đ' => out.push('d'),
            'ø' => out.push('o'),
            'ı' => out.push('i'),
            'ß' => out.push_str("ss"),
            'æ' => out.push_str("ae"),
            'œ' => out.push_str("oe"),
            _ => out.push(chars::normalize(c)),
        }
    }
    out
}

/// A typed query, matched against many rows.
pub struct Query {
    matcher: Matcher,
    pattern: Option<Pattern>,
    buf: Vec<char>,
}

impl std::fmt::Debug for Query {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Query(active = {})", self.pattern.is_some())
    }
}

impl Query {
    pub fn new(query: &str) -> Self {
        let q = fold(query.trim());
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            pattern: (!q.is_empty())
                .then(|| Pattern::new(&q, CaseMatching::Ignore, Normalization::Never, AtomKind::Substring)),
            buf: Vec::new(),
        }
    }

    /// An empty query matches everything.
    pub fn matches(&mut self, haystack: &str) -> bool {
        let Some(pattern) = &self.pattern else { return true };
        let h = fold(haystack);
        pattern.score(Utf32Str::new(&h, &mut self.buf), &mut self.matcher).is_some()
    }
}

/// The rows a query shows: the exact matches in list order; when there is none, the close matches (typos) ranked
/// by (edits, typo words, list order). `close` tells which of the two it is.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub rows: Vec<usize>,
    pub close: bool,
}

/// Filter `haystacks` (one text per row) by `query`, both tiers in one synchronous pass.
pub fn find<I, S>(haystacks: I, query: &str) -> Found
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let texts: Vec<String> = haystacks.into_iter().map(|h| h.as_ref().to_owned()).collect();
    let mut exact = Query::new(query);
    let rows: Vec<usize> = (0..texts.len()).filter(|&i| exact.matches(&texts[i])).collect();
    if !rows.is_empty() || query.trim().is_empty() {
        return Found { rows, close: false };
    }
    let words: Vec<String> = fold(query).split_whitespace().map(str::to_owned).collect();
    let mut close: Vec<(u8, u8, usize)> =
        (0..texts.len()).filter_map(|i| close_match(&fold(&texts[i]), &words).map(|(e, t)| (e, t, i))).collect();
    close.sort_unstable();
    Found { rows: close.into_iter().map(|(_, _, i)| i).collect(), close: true }
}

/// Edits allowed for a typed word: none below 4 letters (2-3 letter typos match noise), 1 up to 7, 2 from 8.
fn allowed(word_len: usize) -> u8 {
    match word_len {
        0..=3 => 0,
        4..=7 => 1,
        _ => 2,
    }
}

const MAX_QUERY_EDITS: u8 = 2;

/// (total edits, typo words) when every word matches the folded text: as a substring, or within `allowed`
/// Damerau-Levenshtein edits of one of its words (the last typed word also of a word's beginning, as it may be
/// unfinished). None otherwise, or past MAX_QUERY_EDITS in total.
fn close_match(text: &str, words: &[String]) -> Option<(u8, u8)> {
    let tokens: Vec<Vec<char>> =
        text.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(|t| t.chars().collect()).collect();
    let (mut edits, mut typos) = (0u8, 0u8);
    for (n, word) in words.iter().enumerate() {
        if text.contains(word.as_str()) {
            continue;
        }
        let w: Vec<char> = word.chars().collect();
        let bound = allowed(w.len());
        if bound == 0 {
            return None;
        }
        let last = n + 1 == words.len();
        let best = tokens
            .iter()
            .flat_map(|t| {
                let whole = std::iter::once(t.as_slice());
                // an unfinished last word: compare with the token's beginning of about the same length
                let prefixes = (w.len().saturating_sub(1)..=w.len() + 1)
                    .filter(move |&k| last && k >= 4 && k < t.len())
                    .map(move |k| &t[..k]);
                whole.chain(prefixes)
            })
            .filter_map(|t| osa_distance(&w, t, bound))
            .min()?;
        edits += best;
        typos += 1;
        if edits > MAX_QUERY_EDITS {
            return None;
        }
    }
    Some((edits, typos))
}

/// Optimal string alignment distance (Damerau-Levenshtein where an adjacent swap is one edit), or None when it
/// is above `bound`.
fn osa_distance(a: &[char], b: &[char], bound: u8) -> Option<u8> {
    let bound = usize::from(bound);
    if a.len().abs_diff(b.len()) > bound {
        return None;
    }
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=m {
        d[0][j] = j;
    }
    for i in 1..=n {
        let mut row_min = usize::MAX;
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
            row_min = row_min.min(v);
        }
        if row_min > bound {
            return None;
        }
    }
    (d[n][m] <= bound).then(|| d[n][m] as u8)
}

/// Up/Down and Ctrl-n/Ctrl-p while a filter takes text: Some(true) moves down, Some(false) up.
pub fn nav_key(key: &Key) -> Option<bool> {
    use crossterm::event::{KeyCode, KeyModifiers};
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.key {
        KeyCode::Down => Some(true),
        KeyCode::Up => Some(false),
        KeyCode::Char('n') if ctrl => Some(true),
        KeyCode::Char('p') if ctrl => Some(false),
        _ => None,
    }
}

/// The key bound to an action, as a hint shows it: single keys first, Enter/Esc before letters before the rest;
/// None when the action has no binding.
pub fn binding<A>(map: &HashMap<KeySequence, A>, want: impl Fn(&A) -> bool) -> Option<String> {
    use crossterm::event::{KeyCode, KeyModifiers};
    map.iter()
        .filter(|(_, a)| want(a))
        .map(|(seq, _)| {
            let rank = match seq.0.as_slice() {
                [k] if matches!(k.key, KeyCode::Enter | KeyCode::Esc) && k.modifiers == KeyModifiers::NONE => 0,
                [k] if matches!(k.key, KeyCode::Char(_)) && k.modifiers == KeyModifiers::NONE => 1,
                [_] => 2,
                _ => 3,
            };
            (rank, seq.0.iter().map(key_label).collect::<String>())
        })
        .min()
        .map(|(_, label)| label)
}

/// The text of a key as the hint line shows it ("Enter", "/", "Space", "Ctrl-x").
pub fn key_label(key: &Key) -> String {
    use crossterm::event::{KeyCode, KeyModifiers};
    let base = match key.key {
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) => c.to_string(),
        KeyCode::Enter => "Enter".to_owned(),
        KeyCode::Esc => "Esc".to_owned(),
        KeyCode::Tab => "Tab".to_owned(),
        KeyCode::BackTab => "Shift-Tab".to_owned(),
        KeyCode::Up => "↑".to_owned(),
        KeyCode::Down => "↓".to_owned(),
        KeyCode::Left => "←".to_owned(),
        KeyCode::Right => "→".to_owned(),
        other => format!("{other:?}"),
    };
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        format!("Ctrl-{base}")
    } else if key.modifiers.contains(KeyModifiers::ALT) {
        format!("Alt-{base}")
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folds_polish_and_other_letters() {
        assert_eq!(fold("Żółw"), "zolw");
        assert_eq!(fold("Łódź"), "lodz");
        assert_eq!(fold("Zażółć gęślą jaźń"), "zazolc gesla jazn");
        assert_eq!(fold("Straße Ærø"), "strasse aero");
    }

    #[test]
    fn words_match_folded_text_in_any_order() {
        assert!(Query::new("zolw").matches("Żółw - Piosenka"));
        assert!(Query::new("LODZ").matches("Ballada o Łodzi Łódź"));
        assert!(Query::new("strings tiesto").matches("Tiësto - Adagio For Strings"));
        assert!(!Query::new("lodz").matches("Myslovitz - Długość dźwięku samotności")); // no scattered letters
        assert!(!Query::new("tiesto xyz").matches("Tiësto - Adagio For Strings"));
        assert!(Query::new("  ").matches("anything"));
    }

    const ROWS: [&str; 7] = [
        "Beyoncé - Halo - I Am... Sasha Fierce - beyonce-halo.mp3",
        "Metallica - Nothing Else Matters - Metallica - 01.flac",
        "Kavinsky - Nightcall - OutRun - nightcall.mp3",
        "Lady Pank - Zawsze tam gdzie ty - Tacy sami - 092.mp3",
        "Myslovitz - Długość dźwięku samotności - Miłość w czasach popkultury - 07.mp3",
        "Kygo - Firestone - Cloud Nine - 081.mp3",
        "ABBA - SOS - ABBA - 03.mp3",
    ];

    fn close(q: &str) -> Found {
        find(ROWS, q)
    }

    #[test]
    fn typos_find_close_matches_when_nothing_matches_exactly() {
        assert_eq!(close("beyonse"), Found { rows: vec![0], close: true });
        assert_eq!(close("metalika"), Found { rows: vec![1], close: true });
        assert_eq!(close("metallca"), Found { rows: vec![1], close: true });
        assert_eq!(close("kawinsky"), Found { rows: vec![2], close: true });
        assert_eq!(close("zaowsze"), Found { rows: vec![3], close: true });
        assert_eq!(close("nigthcall"), Found { rows: vec![2], close: true }); // transposition
        assert_eq!(close("kavinsky nigthc"), Found { rows: vec![2], close: true }); // unfinished last word
    }

    #[test]
    fn close_matches_stay_strict() {
        assert_eq!(close("lodz"), Found { rows: vec![], close: true }); // no scattered letters, no "lody"
        assert_eq!(close("sus"), Found { rows: vec![], close: true }); // 3 letters: strict ("sos" is one edit)
        assert_eq!(close("metalika nithgcal"), Found { rows: vec![], close: true }); // 4 edits in total: too many
        assert_eq!(close("halo"), Found { rows: vec![0], close: false }); // exact matches suppress the typos
        assert_eq!(close(""), Found { rows: (0..ROWS.len()).collect(), close: false });
    }

    #[test]
    fn close_matches_rank_by_edits_then_list_order() {
        let rows = ["Kygo - Firestome", "Kygo - Firestone"];
        assert_eq!(find(rows, "firestonr"), Found { rows: vec![1, 0], close: true }); // 1 edit before 2 edits
        assert_eq!(osa_distance(&['a', 'b'], &['b', 'a'], 1), Some(1));
    }
}
