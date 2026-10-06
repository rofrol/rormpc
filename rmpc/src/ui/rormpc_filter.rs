//! rormpc: live inline filtering, shared by the Queue and the Versions pane. Every typed word must appear in the
//! row, in any order (fzf's --exact; whole-row fuzzy matching let "lodz" match l…o…d…z scattered over a long
//! title, too loose for a filter that keeps the list's order). Matching is diacritic-folded, so "zolw" finds
//! "żółw" and "lodz" finds "Łódź"; nucleo matches the needle as given, so both sides are folded here.

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
}
