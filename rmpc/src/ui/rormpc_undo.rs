//! rormpc: undo and redo of Music's filter changes (plans/instant-filtering.md, "Undo instead of the
//! confirmation"; `z` / `Z`, decided by the user 2026-10-11).
//!
//! - Each generation that reached MPD pushes the state it replaced; a generation dropped before it reached MPD
//!   (a newer one came first) is no entry, so a held key is one entry, without any timing.
//! - The state a filter change replaced is the rules then in the queue, or, on the first touch, the other kind
//!   of source it replaced (an album played with `P`, a playlist, the whole library, an unknown queue), with its
//!   files in order: undo brings that source back.
//! - Undo and redo are filter changes themselves (they bump the generation and reconcile); they restore the
//!   membership, never the playback time or a hand edit of the queue. Depth 20; a new change drops the redo.
//! - A hand removal from the queue bans the song (stage 5, decided by the user 2026-10-11), and the ban is an
//!   entry of the same timeline: `z` takes back the newest filter change or ban, whichever came last. Bans are
//!   lasting, so a new source (`reset`) keeps their entries.

/// What a filter change replaced, or what undo goes back to.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Music's filters: a `hits` result's `args` (`HitsPane::load_filters` takes them back)
    Rules(serde_json::Value),
    /// another kind of source the first filter touch replaced
    Other(Departed),
}

/// A ban of a hand removal: its files and what the status says ("Toto - Africa", "3 songs").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ban {
    pub files: Vec<String>,
    pub what: String,
}

/// One entry of the timeline: the filter state a change replaced, or a ban.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    State(State),
    Banned(Ban),
}

/// A source that was not Music's filters, as it was in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Departed {
    /// source.json's kind ("album", "playlist", "library", ...), "selection" for a queue without one
    pub kind: String,
    pub name: String,
    /// its files in queue order (empty for the library and a playlist: they are added whole again)
    pub files: Vec<String>,
}

/// The last generation sent to MPD and not yet known to have reached it.
#[derive(Debug, Clone)]
struct Sent {
    generation: u64,
    state: State,
    /// a user's change pushes what it replaced; an undo or redo has already moved the stacks
    push: bool,
}

/// How many states undo can go back.
pub const DEPTH: usize = 20;

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
    /// the state in the queue as far as the history knows (None: not known yet)
    in_mpd: Option<State>,
    sent: Option<Sent>,
}

impl History {
    /// Another rormpc, Sources… or Browse's `P` put a source in the queue: the old filter history no longer
    /// applies; the bans stay undoable.
    pub fn reset(&mut self, state: Option<State>) {
        let mut undo = std::mem::take(&mut self.undo);
        undo.retain(|e| matches!(e, Entry::Banned(_)));
        *self = Self { undo, in_mpd: state, ..Self::default() };
    }

    /// A hand removal banned `ban`: the newest entry (a change sent before it counts as reached, so it stays the
    /// older one); a new action drops the redo.
    pub fn banned(&mut self, ban: Ban) {
        self.settle(None);
        self.push_undo(Entry::Banned(ban));
        self.redo.clear();
    }

    fn push_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
        if self.undo.len() > DEPTH {
            self.undo.remove(0);
        }
    }

    /// The queue holds `state` now (another kind of source, about to be replaced by the filters).
    pub fn set_in_mpd(&mut self, state: State) {
        self.in_mpd = Some(state);
    }

    /// The state undo and redo start from: the newest one sent, else the one in the queue.
    pub fn current(&self) -> Option<&State> {
        self.sent.as_ref().map(|s| &s.state).or(self.in_mpd.as_ref())
    }

    /// Generation `generation` with `state` was sent to MPD; `reached` is the newest generation that reached it.
    /// `push`: a user's change (it records what it replaced once it reaches MPD), not an undo or redo.
    pub fn sent(&mut self, generation: u64, state: State, push: bool, reached: u64) {
        self.settle(Some(reached));
        self.sent = Some(Sent { generation, state, push });
    }

    /// The generation sent last reached MPD (`reached` at least its own; None: take it as reached): it becomes
    /// the state in the queue, and a user's change records the one it replaced. One that never reached MPD
    /// (a newer generation dropped it) is forgotten.
    fn settle(&mut self, reached: Option<u64>) {
        let Some(s) = self.sent.take() else { return };
        if reached.is_some_and(|r| r < s.generation) {
            return;
        }
        if s.push {
            match self.in_mpd.take() {
                Some(prev) if prev != s.state => {
                    self.push_undo(Entry::State(prev));
                    self.redo.clear();
                }
                _ => {}
            }
        }
        self.in_mpd = Some(s.state);
    }

    /// `z`: the state to go back to (the caller restores it as a filter change sent with `push` false), or the
    /// ban to take back.
    pub fn undo(&mut self) -> Option<Entry> {
        self.settle(None);
        let target = self.undo.pop()?;
        if let Some(back) = self.step_to(&target) {
            self.redo.push(back);
        }
        Some(target)
    }

    /// `Z`: the state an undo left, back again, or the ban an undo took back.
    pub fn redo(&mut self) -> Option<Entry> {
        self.settle(None);
        let target = self.redo.pop()?;
        if let Some(back) = self.step_to(&target) {
            self.undo.push(back);
        }
        Some(target)
    }

    /// Go to `target`: the entry that takes the step back (the state left, or the same ban).
    fn step_to(&mut self, target: &Entry) -> Option<Entry> {
        match target {
            Entry::State(state) => self.in_mpd.replace(state.clone()).map(Entry::State),
            Entry::Banned(ban) => Some(Entry::Banned(ban.clone())),
        }
    }

    /// Whether `z` / `Z` have something to do (the generation sent last counts as reached).
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.sent.as_ref().is_some_and(|s| s.push && self.in_mpd.as_ref().is_some_and(|p| *p != s.state))
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() && !self.sent.as_ref().is_some_and(|s| s.push)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(n: u64) -> State {
        State::Rules(serde_json::json!({ "n": n }))
    }

    fn album() -> State {
        State::Other(Departed { kind: "album".into(), name: "Low".into(), files: vec!["b.flac".into(), "a.flac".into()] })
    }

    #[test]
    fn each_change_that_reached_mpd_is_one_entry_and_undo_redo_walk_them() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        h.sent(1, rules(1), true, 0);
        h.sent(2, rules(2), true, 1);
        assert!(h.can_undo());
        assert!(!h.can_redo());
        assert_eq!(h.undo(), Some(Entry::State(rules(1))));
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        assert_eq!(h.undo(), None);
        assert!(h.can_redo());
        assert_eq!(h.redo(), Some(Entry::State(rules(1))));
        assert_eq!(h.redo(), Some(Entry::State(rules(2))));
        assert_eq!(h.redo(), None);
    }

    #[test]
    fn a_generation_dropped_before_it_reached_mpd_is_no_entry() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        // a held key: 1 was sent, 2 came before 1 was dequeued (1 never reached MPD), 3 after 2 reached it
        h.sent(1, rules(1), true, 0);
        h.sent(2, rules(2), true, 0);
        h.sent(3, rules(3), true, 2);
        assert_eq!(h.undo(), Some(Entry::State(rules(2))));
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        assert_eq!(h.undo(), None);
    }

    #[test]
    fn undo_and_redo_are_not_entries_themselves_and_a_new_change_drops_the_redo() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        h.sent(1, rules(1), true, 0);
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        h.sent(2, rules(0), false, 1); // the undo's own generation
        assert!(h.can_redo());
        h.sent(3, rules(5), true, 2);
        assert!(!h.can_redo(), "a new change drops the redo");
        assert_eq!(h.redo(), None);
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        assert_eq!(h.undo(), None);
    }

    #[test]
    fn quick_undos_before_their_runs_finish_go_back_one_step_each() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        h.sent(1, rules(1), true, 0);
        h.sent(2, rules(2), true, 1);
        h.sent(3, rules(3), true, 2);
        assert_eq!(h.undo(), Some(Entry::State(rules(2))));
        assert_eq!(h.undo(), Some(Entry::State(rules(1))));
        assert_eq!(h.redo(), Some(Entry::State(rules(2))));
        assert_eq!(h.redo(), Some(Entry::State(rules(3))));
    }

    #[test]
    fn the_first_touch_after_another_source_undoes_to_it() {
        let mut h = History::default();
        h.reset(None);
        h.set_in_mpd(album());
        h.sent(1, rules(1), true, 0);
        h.sent(2, rules(2), true, 1);
        assert_eq!(h.undo(), Some(Entry::State(rules(1))));
        assert_eq!(h.undo(), Some(Entry::State(album())));
        assert_eq!(h.current(), Some(&album()));
        assert_eq!(h.redo(), Some(Entry::State(rules(1))));
    }

    #[test]
    fn the_history_keeps_the_last_twenty_states() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        for g in 1..=30 {
            h.sent(g, rules(g), true, g - 1);
        }
        let mut back = 0;
        while h.undo().is_some() {
            back += 1;
        }
        assert_eq!(back, DEPTH);
        assert_eq!(h.current(), Some(&rules(10)));
    }

    #[test]
    fn the_same_rules_again_are_no_entry_and_a_reset_forgets_everything() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        h.sent(1, rules(0), true, 0);
        h.sent(2, rules(1), true, 1);
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        assert_eq!(h.undo(), None);
        h.reset(Some(rules(7)));
        assert!(!h.can_undo() && !h.can_redo());
    }

    fn ban(file: &str) -> Ban {
        Ban { files: vec![file.to_owned()], what: file.to_owned() }
    }

    #[test]
    fn bans_and_filter_changes_share_one_timeline() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        h.sent(1, rules(1), true, 0);
        h.banned(ban("a.mp3"));
        h.sent(2, rules(2), true, 1);
        assert_eq!(h.undo(), Some(Entry::State(rules(1))));
        assert_eq!(h.undo(), Some(Entry::Banned(ban("a.mp3"))), "the ban came between the two changes");
        assert_eq!(h.current(), Some(&rules(1)), "taking a ban back leaves the filters as they are");
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        assert_eq!(h.redo(), Some(Entry::State(rules(1))));
        assert_eq!(h.redo(), Some(Entry::Banned(ban("a.mp3"))));
        assert_eq!(h.redo(), Some(Entry::State(rules(2))));
        assert_eq!(h.redo(), None);
    }

    #[test]
    fn a_ban_drops_the_redo_and_a_new_source_keeps_the_bans() {
        let mut h = History::default();
        h.reset(Some(rules(0)));
        h.sent(1, rules(1), true, 0);
        assert_eq!(h.undo(), Some(Entry::State(rules(0))));
        h.banned(ban("a.mp3"));
        assert!(!h.can_redo());
        h.reset(None);
        assert!(h.can_undo());
        assert_eq!(h.undo(), Some(Entry::Banned(ban("a.mp3"))));
        assert_eq!(h.undo(), None);
        assert_eq!(h.redo(), Some(Entry::Banned(ban("a.mp3"))));
    }
}
