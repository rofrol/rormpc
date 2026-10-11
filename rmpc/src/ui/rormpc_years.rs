//! rormpc: the "Years to review" panel over Music (`gY`, `ShowPlay(Years)`; rormpc-tools
//! docs/release-years-plan.md, stage B). It reads `musicdb years --json` (the dry-run report, version 1): one row
//! per file whose shown year (`OriginalDate`, else `Date`) would change or that is suspect, with the current and
//! proposed TDRC/TDOR, the rule, the confidence, the evidence and the decision. Every row is reviewed (decided by
//! the user 2026-10-10): `a` accepts, `D` rejects, the same key again on rows already so decided makes them
//! undecided (`musicdb years --accept/--reject/--undecide ID...`); Space marks rows and the keys act on the marked
//! ones. "Apply accepted…" (`musicdb years --apply`) and "Roll back…" (`--rollback [ID...]`) run in the background
//! after a confirmation and show their result; the writing, its checks and the backup stay in rormpc-tools.
//! Rows that need an MBID (no `MusicBrainz` recording or work) get the MBID picker (stage B2): `a` on such a row, or
//! "Pick the recording…" in the menu on any row not applied, asks for a `MusicBrainz` recording URL or MBID (the
//! CLI offers no candidate list) and runs `musicdb years --mbid ID MBID` in the background; the row comes back
//! computed from the pick, undecided. Applying an accepted pick also writes it into the file's recording tag (UFID /
//! `MUSICBRAINZ_TRACKID`, even when the year holds; decided by the user 2026-10-11), and Roll back… restores it.
//! h/l switch the decision filter; the menu (Enter) also filters by confidence and class. Files without a recording
//! MBID (class `no-recording`) show only under their own menu filter, off by default (decided by the user 2026-10-11).

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    sync::{Arc, Mutex},
};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
};
use rmpc_mpd::commands::Song;
use serde::Deserialize;

use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        macros::{modal, status_error, status_info, status_warn},
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        UiEvent,
        dirstack::DirState,
        modals::{
            confirm_modal::{Action, ConfirmModal},
            input_modal::InputModal,
            menu::modal::MenuModal,
        },
        panes::{Pane, deleted},
    },
};

/// The `years-report.json` version this rormpc reads (rormpc-tools `years.REPORT_VERSION`).
const REPORT_VERSION: u32 = 1;
/// From this panel width on, the details sit beside the table instead of under it.
const WIDE: u16 = 150;

/// The date the views show for a song: MPD's `originaldate` (TDOR, the song's original release), falling back to
/// `date` (TDRC, this recording's own release). The album browser does the same through `album_date_tags`.
pub fn shown_date(song: &Song) -> String {
    ["originaldate", "date"]
        .iter()
        .find_map(|tag| song.metadata.get(*tag).map(|t| t.first().to_owned()).filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Report {
    version: u32,
    #[serde(default)]
    generated: Option<String>,
    #[serde(default)]
    rows: Vec<YearRow>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
struct Dates {
    #[serde(rename = "TDRC", default)]
    tdrc: Option<String>,
    #[serde(rename = "TDOR", default)]
    tdor: Option<String>,
    #[serde(rename = "DATE_SOURCE", default)]
    source: Option<String>,
    #[serde(rename = "DATE_RULE", default)]
    rule: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Release {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct Detail {
    #[serde(default)]
    matched: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    release: Option<Release>,
    #[serde(default)]
    rg_date: Option<String>,
    #[serde(default)]
    tdrc_own: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct YearRow {
    id: u64,
    file: String,
    #[serde(default)]
    mbid: Option<String>,
    #[serde(default)]
    mbid_override: Option<String>,
    #[serde(default)]
    current: Dates,
    #[serde(default)]
    proposed: Option<Dates>,
    #[serde(default)]
    rule: Option<String>,
    /// "high", "review", or none without a proposal
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    classes: Vec<String>,
    #[serde(default)]
    evidence: String,
    #[serde(default)]
    detail: Option<Detail>,
    #[serde(default)]
    needs_mbid: bool,
    /// undecided, accepted, rejected
    decision: String,
    #[serde(default)]
    decided: Option<String>,
    /// when `--apply` wrote the row
    #[serde(default)]
    applied: Option<String>,
    /// the file's recording tag when `--mbid` picked one (null: the file has none); `--apply` writes the pick over it
    #[serde(default)]
    file_mbid: Option<String>,
}

impl YearRow {
    /// Read-only here: rows that need an MBID and have no pick yet (the picker first) and rows already written
    /// (Roll back… first).
    fn locked(&self) -> Option<&'static str> {
        if self.applied.is_some() {
            Some("already applied: Roll back… (menu) first")
        } else if self.wants_pick() {
            Some("needs an MBID: a or Enter → Pick the recording… (a MusicBrainz recording URL or MBID)")
        } else {
            None
        }
    }

    /// A row that needs an MBID and has none picked yet.
    fn wants_pick(&self) -> bool {
        self.needs_mbid && self.mbid_override.is_none()
    }

    /// Whether `--accept` takes the row (rormpc-tools `acceptable`): a proposal, or a recording picked by hand that
    /// `MusicBrainz` knows (its tag is written even when the year holds).
    fn acceptable(&self) -> bool {
        self.proposed.is_some()
            || self.mbid_override.is_some() && !self.classes.iter().any(|c| c == "recording-not-found")
    }

    /// The picked recording when applying the row rewrites the file's recording tag: (old tag, pick).
    fn rewrites_recording(&self) -> Option<(Option<&str>, &str)> {
        let pick = self.mbid_override.as_deref()?;
        (self.file_mbid.as_deref() != Some(pick)).then_some((self.file_mbid.as_deref(), pick))
    }

    /// A file without a `MusicBrainz` recording MBID (class `no-recording`, rormpc-tools 0.2.54): listed only under
    /// its own filter (decided by the user 2026-10-11). A pick replaces the class, so the row joins the others.
    fn no_recording(&self) -> bool {
        self.classes.iter().any(|c| c == "no-recording")
    }

    fn confidence_key(&self) -> &str {
        self.confidence.as_deref().unwrap_or("none")
    }

    /// The table's short evidence: the suspect classes and the source release.
    fn short_evidence(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(pick) = &self.mbid_override {
            parts.push(format!("picked {}", short(pick)));
        } else if self.needs_mbid {
            parts.push("needs MBID · a: pick".to_owned());
        }
        let classes: Vec<&str> =
            self.classes.iter().map(String::as_str).filter(|c| !(self.needs_mbid && *c == "no-work")).collect();
        if !classes.is_empty() {
            parts.push(classes.join(", "));
        }
        if let Some(rel) = self.detail.as_ref().and_then(|d| d.release.as_ref()) {
            parts.push(format!(
                "{} \"{}\"",
                rel.kind.as_deref().unwrap_or("?"),
                rel.title.as_deref().unwrap_or("?")
            ));
        }
        if parts.is_empty() {
            self.evidence.clone()
        } else {
            parts.join(" · ")
        }
    }
}

fn dates_cell(d: Option<&Dates>) -> String {
    let v = |x: Option<&String>| x.map_or("–", |s| s.as_str()).to_owned();
    match d {
        Some(d) => format!("{} / {}", v(d.tdrc.as_ref()), v(d.tdor.as_ref())),
        None => "–".to_owned(),
    }
}

/// An MBID's first 8 characters, as rormpc-tools shows them.
fn short(mbid: &str) -> &str {
    mbid.get(..8).unwrap_or(mbid)
}

/// The recording MBID in what was pasted into the picker: a bare MBID or a `MusicBrainz` recording URL
/// (`https://musicbrainz.org/recording/<MBID>`, with or without a trailing path or query), lowercased as
/// `musicdb years --mbid` checks it. Other entity URLs (a release, a work) are said as such.
fn parse_mbid(input: &str) -> Result<String, String> {
    let text = input.trim();
    let is_mbid = |s: &str| {
        s.len() == 36
            && s.char_indices().all(|(i, c)| if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() })
    };
    if is_mbid(text) {
        return Ok(text.to_ascii_lowercase());
    }
    let path = text.split(['?', '#']).next().unwrap_or_default();
    let mut parts = path.split('/').filter(|p| !p.is_empty()).skip_while(|p| !p.contains("musicbrainz.org"));
    if parts.next().is_none() {
        return Err(format!("not a MusicBrainz recording URL or MBID: {text}"));
    }
    match (parts.next(), parts.next()) {
        (Some("recording"), Some(id)) if is_mbid(id) => Ok(id.to_ascii_lowercase()),
        (Some("recording"), _) => Err(format!("no recording MBID in this URL: {text}")),
        (Some(kind), _) => Err(format!("a {kind} URL, not a recording: open the recording on MusicBrainz and paste its URL")),
        (None, _) => Err(format!("no recording MBID in this URL: {text}")),
    }
}

/// `musicdb years --mbid ID MBID`
fn mbid_args(id: u64, mbid: &str) -> Vec<String> {
    vec!["years".to_owned(), "--mbid".to_owned(), id.to_string(), mbid.to_owned()]
}

/// `--mbid` output (the row computed again from the pick) as the status line.
fn picked_message(id: u64, mbid: &str, json: &str) -> String {
    let mut text = format!("Years: #{id} picked {}", short(mbid));
    if let Ok(r) = serde_json::from_str::<YearRow>(json.trim()) {
        match &r.proposed {
            Some(p) => {
                let _ = write!(text, " · proposed {}", dates_cell(Some(p)));
            }
            None if r.classes.iter().any(|c| c == "recording-not-found") => {
                text.push_str(" · MusicBrainz does not know this recording: it cannot be accepted");
            }
            None => text.push_str(" · no new year"),
        }
        text.push_str(" · undecided: a accepts (applying writes the recording tag)");
    }
    text
}

fn parse_report(json: &str) -> Result<Report, String> {
    let report: Report = serde_json::from_str(json).map_err(|e| format!("unreadable years report: {e}"))?;
    if report.version != REPORT_VERSION {
        return Err(format!(
            "years report version {} (this rormpc reads {REPORT_VERSION}): update rormpc and rormpc-tools together",
            report.version
        ));
    }
    Ok(report)
}

/// A decision key: `a` accepts, `D` rejects; the same key again undecides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decision {
    Accepted,
    Rejected,
    Undecided,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::Accepted => "accepted",
            Decision::Rejected => "rejected",
            Decision::Undecided => "undecided",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Decision::Accepted => "--accept",
            Decision::Rejected => "--reject",
            Decision::Undecided => "--undecide",
        }
    }
}

/// `musicdb years --accept|--reject|--undecide ID...`
fn decide_args(decision: Decision, ids: &[u64]) -> Vec<String> {
    let mut args = vec!["years".to_owned(), decision.flag().to_owned()];
    args.extend(ids.iter().map(u64::to_string));
    args
}

/// `musicdb years --rollback [ID...]`: no ids rolls back the newest apply of every file.
fn rollback_args(ids: &[u64]) -> Vec<String> {
    let mut args = vec!["years".to_owned(), "--rollback".to_owned()];
    args.extend(ids.iter().map(u64::to_string));
    args
}

const LOAD_ARGS: [&str; 2] = ["years", "--json"];
const APPLY_ARGS: [&str; 2] = ["years", "--apply"];

/// What a decision key does to these rows: the key's decision, or undecided when every row already has it.
fn toggled(key: Decision, rows: &[&YearRow]) -> Decision {
    if !rows.is_empty() && rows.iter().all(|r| r.decision == key.as_str()) { Decision::Undecided } else { key }
}

/// `--apply` / `--rollback` output as the result dialog's text.
fn outcome_message(verb: &str, json: &str) -> String {
    /// the recording tag an apply of a pick (or its rollback) rewrote; null: none
    #[derive(Deserialize)]
    struct Recording {
        old: Option<String>,
        new: Option<String>,
    }
    #[derive(Deserialize)]
    struct Done {
        id: u64,
        file: String,
        #[serde(default)]
        recording: Option<Recording>,
    }
    #[derive(Deserialize)]
    struct Skipped {
        id: u64,
        file: String,
        why: String,
    }
    #[derive(Deserialize)]
    struct Outcome {
        #[serde(default, alias = "rolled_back")]
        applied: Vec<Done>,
        #[serde(default)]
        skipped: Vec<Skipped>,
    }
    let Ok(out) = serde_json::from_str::<Outcome>(json.trim()) else {
        return format!("{verb}: {}", json.trim());
    };
    let rows = |n: usize| if n == 1 { "row" } else { "rows" };
    let mut text = format!("{verb} {} {}.", out.applied.len(), rows(out.applied.len()));
    for d in &out.applied {
        let _ = write!(text, "\n  #{} {}", d.id, d.file);
        if let Some(rec) = &d.recording {
            let tag = |m: Option<&String>| m.map_or("none", |m| short(m)).to_owned();
            let _ = write!(text, " · recording tag {} → {}", tag(rec.old.as_ref()), tag(rec.new.as_ref()));
        }
    }
    if !out.skipped.is_empty() {
        let _ = write!(text, "\n\nSkipped {} {}:", out.skipped.len(), rows(out.skipped.len()));
        for s in &out.skipped {
            let _ = write!(text, "\n  #{} {}: {}", s.id, s.file, s.why);
        }
    }
    text
}

/// The load error as the panel says it.
fn load_error(err: String) -> String {
    if err.contains("invalid choice: 'years'") {
        "this musicdb has no `years` command: install a rormpc-tools with release years (stage A)".to_owned()
    } else {
        err
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DecisionFilter {
    All,
    #[default]
    Undecided,
    Accepted,
    Rejected,
}

impl DecisionFilter {
    const ORDER: [DecisionFilter; 4] =
        [DecisionFilter::Undecided, DecisionFilter::Accepted, DecisionFilter::Rejected, DecisionFilter::All];

    fn label(self) -> &'static str {
        match self {
            DecisionFilter::All => "all",
            DecisionFilter::Undecided => "undecided",
            DecisionFilter::Accepted => "accepted",
            DecisionFilter::Rejected => "rejected",
        }
    }

    fn step(self, forward: bool) -> Self {
        let i = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        let n = Self::ORDER.len();
        Self::ORDER[if forward { (i + 1) % n } else { (i + n - 1) % n }]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Filter {
    decision: DecisionFilter,
    /// high, review, none
    confidence: Option<String>,
    class: Option<String>,
    /// on: only the files without a recording MBID; off (the default): every other row
    no_recording: bool,
}

impl Filter {
    fn matches(&self, r: &YearRow) -> bool {
        r.no_recording() == self.no_recording
            && (self.decision == DecisionFilter::All || r.decision == self.decision.label())
            && self.confidence.as_ref().is_none_or(|c| r.confidence_key() == c)
            && self.class.as_ref().is_none_or(|c| r.classes.contains(c))
    }

    fn describe(&self) -> String {
        let mut out = self.decision.label().to_owned();
        if self.no_recording {
            out.push_str(" · no recording MBID");
        }
        if let Some(c) = &self.confidence {
            let _ = write!(out, " · confidence {c}");
        }
        if let Some(c) = &self.class {
            let _ = write!(out, " · class {c}");
        }
        out
    }
}

/// The counts at the top, over every row of the report.
fn summary(rows: &[YearRow]) -> String {
    let count = |f: &dyn Fn(&YearRow) -> bool| rows.iter().filter(|r| f(r)).count();
    format!(
        "{} rows · {} undecided · {} accepted · {} rejected · {} applied · {} high · {} review · {} need an MBID · {} \
         picked · {} without a recording MBID",
        rows.len(),
        count(&|r| r.decision == "undecided"),
        count(&|r| r.decision == "accepted"),
        count(&|r| r.decision == "rejected"),
        count(&|r| r.applied.is_some()),
        count(&|r| r.confidence.as_deref() == Some("high")),
        count(&|r| r.confidence.as_deref() == Some("review")),
        count(&|r| r.wants_pick()),
        count(&|r| r.mbid_override.is_some()),
        count(&|r| r.no_recording()),
    )
}

/// The menu's filter counts: classes and confidences over the rows on the same side of the "no recording MBID"
/// filter as `no_recording` (the hidden side never counts), and how many files have no recording MBID.
fn filter_counts(rows: &[YearRow], no_recording: bool) -> (BTreeMap<String, usize>, BTreeMap<String, usize>, usize) {
    let mut classes: BTreeMap<String, usize> = BTreeMap::new();
    let mut confidences: BTreeMap<String, usize> = BTreeMap::new();
    for r in rows.iter().filter(|r| r.no_recording() == no_recording) {
        // the class has its own filter entry
        for c in r.classes.iter().filter(|c| *c != "no-recording") {
            *classes.entry(c.clone()).or_default() += 1;
        }
        *confidences.entry(r.confidence_key().to_owned()).or_default() += 1;
    }
    (classes, confidences, rows.iter().filter(|r| r.no_recording()).count())
}

/// State shared with the background `musicdb` runs and the menu's callbacks.
#[derive(Debug, Default)]
struct Job {
    loading: bool,
    /// a newer report to take on the next render
    report: Option<Report>,
    error: Option<String>,
    /// a decision, apply or rollback finished: load again
    reload: bool,
    /// what runs in the background (one write at a time)
    busy: Option<&'static str>,
    /// an apply's or rollback's result, shown on the next render (a modal needs the render's ctx)
    result: Option<String>,
    /// filters and actions picked in the menu, taken on the next render or key
    filter: Option<Filter>,
    decide: Option<Decision>,
    confirm: Option<Confirm>,
    /// the row whose recording to pick (the menu's "Pick the recording…")
    pick: Option<u64>,
}

#[derive(Debug, Clone)]
enum Confirm {
    Apply,
    Rollback(Vec<u64>),
}

#[derive(Debug)]
pub struct YearsPane {
    report: Report,
    /// indices into `report.rows` that pass the filter, in report order
    shown: Vec<usize>,
    filter: Filter,
    marked: BTreeSet<u64>,
    state: DirState<TableState>,
    table_area: Rect,
    job: Arc<Mutex<Job>>,
}

impl YearsPane {
    pub fn new() -> Self {
        Self {
            report: Report::default(),
            shown: Vec::new(),
            filter: Filter::default(),
            marked: BTreeSet::new(),
            state: DirState::default(),
            table_area: Rect::default(),
            job: Arc::new(Mutex::new(Job::default())),
        }
    }

    pub fn has_marks(&self) -> bool {
        !self.marked.is_empty()
    }

    fn load(&self, ctx: &Ctx) {
        let mut job = self.job.lock().expect("years job lock");
        if job.loading {
            return;
        }
        job.loading = true;
        drop(job);
        let (job, sender) = (Arc::clone(&self.job), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let result = deleted::run(&LOAD_ARGS).map_err(load_error).and_then(|out| parse_report(&out));
            let mut j = job.lock().expect("years job lock");
            j.loading = false;
            match result {
                Ok(report) => {
                    j.report = Some(report);
                    j.error = None;
                }
                Err(err) => j.error = Some(err),
            }
            drop(j);
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// Recompute the filtered rows, keeping the selected row (or its position when it left the filter). Marks
    /// stay only on shown rows: the keys never act on rows the filter hides.
    fn refilter(&mut self) {
        let keep = self.selected().map(|r| r.id);
        let at = self.state.get_selected().unwrap_or(0);
        self.shown = (0..self.report.rows.len()).filter(|&i| self.filter.matches(&self.report.rows[i])).collect();
        let ids: BTreeSet<u64> = self.shown.iter().map(|&i| self.report.rows[i].id).collect();
        self.marked.retain(|id| ids.contains(id));
        self.state.set_content_and_viewport_len(self.shown.len(), self.table_area.height.saturating_sub(1).into());
        let idx = keep
            .and_then(|id| self.shown.iter().position(|&i| self.report.rows[i].id == id))
            .unwrap_or_else(|| at.min(self.shown.len().saturating_sub(1)));
        self.state.select((!self.shown.is_empty()).then_some(idx), 0);
    }

    /// Take what the background runs and the menu left. Returns whether a load runs, its error and what runs.
    fn refresh(&mut self, ctx: &Ctx) -> (bool, Option<String>, Option<&'static str>) {
        let (fresh, reload, loading, error, busy, result, filter, decide, confirm, pick) = {
            let mut j = self.job.lock().expect("years job lock");
            (
                j.report.take(),
                std::mem::take(&mut j.reload),
                j.loading,
                j.error.clone(),
                j.busy,
                j.result.take(),
                j.filter.take(),
                j.decide.take(),
                j.confirm.take(),
                j.pick.take(),
            )
        };
        if reload {
            self.load(ctx);
        }
        let mut changed = false;
        if let Some(report) = fresh {
            self.report = report;
            changed = true;
        }
        if let Some(filter) = filter {
            self.filter = filter;
            changed = true;
        }
        if changed {
            self.refilter();
        }
        if let Some(text) = result {
            modal!(
                ctx,
                ConfirmModal::builder()
                    .ctx(ctx)
                    .message(vec![text])
                    .action(Action::CustomButtons { buttons: vec![("Close", Box::new(|_: &Ctx| Ok(())))] })
                    .build()
            );
        }
        if let Some(d) = decide {
            self.decide(d, ctx);
        }
        match confirm {
            Some(Confirm::Apply) => self.confirm_apply(ctx),
            Some(Confirm::Rollback(ids)) => self.confirm_rollback(ctx, ids),
            None => {}
        }
        if let Some(id) = pick {
            self.pick(ctx, id);
        }
        (loading, error, busy)
    }

    fn selected(&self) -> Option<&YearRow> {
        self.state.get_selected().and_then(|i| self.shown.get(i)).and_then(|&i| self.report.rows.get(i))
    }

    /// The marked rows in report order, else the row under the cursor.
    fn targets(&self) -> Vec<&YearRow> {
        if self.marked.is_empty() {
            self.selected().into_iter().collect()
        } else {
            self.report.rows.iter().filter(|r| self.marked.contains(&r.id)).collect()
        }
    }

    /// `a` / `D` (or the menu): decide the targets; rows that cannot take the decision are left out and said.
    fn decide(&mut self, key: Decision, ctx: &Ctx) {
        let targets = self.targets();
        if targets.is_empty() {
            return;
        }
        if let [one] = targets.as_slice()
            && key == Decision::Accepted
            && one.applied.is_none()
            && one.wants_pick()
        {
            let id = one.id;
            self.pick(ctx, id);
            return;
        }
        let (open, locked): (Vec<&YearRow>, Vec<&YearRow>) = targets.into_iter().partition(|r| r.locked().is_none());
        let decision = if key == Decision::Undecided { key } else { toggled(key, &open) };
        let (ok, no_proposal): (Vec<&YearRow>, Vec<&YearRow>) =
            open.into_iter().partition(|r| decision != Decision::Accepted || r.acceptable());
        let mut notes = Vec::new();
        if let [one] = locked.as_slice() {
            notes.push(format!("#{} {}", one.id, one.locked().unwrap_or_default()));
        } else if !locked.is_empty() {
            notes.push(format!("{} rows left out (need an MBID picked or already applied)", locked.len()));
        }
        if !no_proposal.is_empty() {
            notes.push(format!("{} rows have no proposal or known picked recording to accept", no_proposal.len()));
        }
        let ids: Vec<u64> = ok.iter().map(|r| r.id).collect();
        if !notes.is_empty() {
            status_info!("{}", notes.join(" · "));
        }
        if ids.is_empty() {
            return;
        }
        self.marked.clear();
        let what = format!("{} {}", decision.as_str(), if ids.len() == 1 { format!("#{}", ids[0]) } else { format!("{} rows", ids.len()) });
        run_then_reload(ctx, &self.job, decide_args(decision, &ids), None, move |_| format!("Years: {what}"));
    }

    fn confirm_apply(&self, ctx: &Ctx) {
        let todo: Vec<&YearRow> = self
            .report
            .rows
            .iter()
            .filter(|r| r.decision == "accepted" && (r.proposed.is_some() || r.mbid_override.is_some()) && r.applied.is_none())
            .collect();
        let n = todo.len();
        if n == 0 {
            status_info!("No accepted rows waiting to be applied");
            return;
        }
        let picks = todo.iter().filter(|r| r.rewrites_recording().is_some()).count();
        let recording = if picks == 0 {
            String::new()
        } else {
            format!(
                "\n\n{picks} of them also get the picked recording written into the file's recording tag (ID3 UFID / \
                 MUSICBRAINZ_TRACKID, what MPD and the scrobbler read), even where the year holds; what is keyed by \
                 the old recording follows the new one."
            )
        };
        let message = format!(
            "Write the years of {n} accepted row{}?{recording}\n\nOnly files whose audio, date tags and recording \
             tag are what the report saw are written (a changed file is skipped and listed). The old values go to \
             years-backup.jsonl first; Roll back… puts them back. MPD is updated afterwards.",
            if n == 1 { "" } else { "s" }
        );
        let job = Arc::clone(&self.job);
        let buttons: Vec<(&str, Box<dyn FnOnce(&Ctx) -> Result<()> + Send + Sync>)> = vec![
            ("Cancel", Box::new(|_: &Ctx| Ok(()))),
            (
                "Apply",
                Box::new(move |ctx: &Ctx| {
                    let args = APPLY_ARGS.iter().map(|a| (*a).to_owned()).collect();
                    run_then_reload(ctx, &job, args, Some("applying"), |out| outcome_message("Applied", out));
                    Ok(())
                }),
            ),
        ];
        modal!(ctx, ConfirmModal::builder().ctx(ctx).message(vec![message]).action(Action::CustomButtons { buttons }).build());
    }

    fn confirm_rollback(&self, ctx: &Ctx, ids: Vec<u64>) {
        let message = if ids.is_empty() {
            "Roll back the newest apply of every file?\n\nEach file gets back the date tags (and the recording tag) \
             the apply replaced, only while it still has what the apply wrote. The rows go back to undecided."
                .to_owned()
        } else {
            format!(
                "Roll back {}?\n\nThe file gets back the date tags (and the recording tag) the apply replaced, only \
                 while it still has what the apply wrote. The row goes back to undecided.",
                ids.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", ")
            )
        };
        let job = Arc::clone(&self.job);
        let buttons: Vec<(&str, Box<dyn FnOnce(&Ctx) -> Result<()> + Send + Sync>)> = vec![
            ("Cancel", Box::new(|_: &Ctx| Ok(()))),
            (
                "Roll back",
                Box::new(move |ctx: &Ctx| {
                    run_then_reload(ctx, &job, rollback_args(&ids), Some("rolling back"), |out| {
                        outcome_message("Rolled back", out)
                    });
                    Ok(())
                }),
            ),
        ];
        modal!(ctx, ConfirmModal::builder().ctx(ctx).message(vec![message]).action(Action::CustomButtons { buttons }).build());
    }

    /// The MBID picker: ask for the recording row `id`'s file is, then `musicdb years --mbid ID MBID` in the
    /// background; the reloaded row is computed from the pick and undecided.
    fn pick(&self, ctx: &Ctx, id: u64) {
        let Some(row) = self.report.rows.iter().find(|r| r.id == id) else {
            return;
        };
        if row.applied.is_some() {
            status_info!("#{id} already applied: Roll back… (menu) first");
            return;
        }
        let have = if row.mbid_override.is_some() { "picked" } else { "in the tags" };
        match row.mbid_override.as_ref().or(row.mbid.as_ref()) {
            Some(m) => status_info!("#{id} {}: recording {have} {m}", row.file),
            None => status_info!("#{id} {}: no recording in the tags", row.file),
        }
        let job = Arc::clone(&self.job);
        modal!(
            ctx,
            InputModal::new(ctx)
                .title("Pick the recording: applying writes it into the file's recording tag")
                .input_label("MusicBrainz recording URL or MBID:")
                .confirm_label("Pick")
                .initial_value(row.mbid_override.clone().unwrap_or_default())
                .on_confirm(move |ctx, value| {
                    match parse_mbid(value) {
                        Ok(mbid) => run_then_reload(ctx, &job, mbid_args(id, &mbid), None, move |out| {
                            picked_message(id, &mbid, out)
                        }),
                        Err(err) => status_warn!("{err}"),
                    }
                    Ok(())
                })
        );
    }

    fn open_menu(&self, ctx: &Ctx) {
        let targets = self.targets();
        let n = targets.len();
        let open = targets.iter().filter(|r| r.locked().is_none()).count();
        let applied: Vec<u64> = targets.iter().filter(|r| r.applied.is_some()).map(|r| r.id).collect();
        let what = if self.marked.is_empty() { "this row".to_owned() } else { format!("{n} marked rows") };
        // one row at a time: the picker names the recording of the row under the cursor (or the one marked row)
        let pick = match targets.as_slice() {
            [one] if one.applied.is_none() => Some((one.id, one.mbid_override.is_some())),
            _ => None,
        };
        let job_p = Arc::clone(&self.job);
        let (job_d, job_f, job_c) = (Arc::clone(&self.job), Arc::clone(&self.job), Arc::clone(&self.job));
        let filter = self.filter.clone();
        let (classes, confidences, no_recording) = filter_counts(&self.report.rows, filter.no_recording);
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
                if let Some((id, picked)) = pick {
                    let label = if picked { "Pick another recording (MBID)…" } else { "Pick the recording (MBID)…" };
                    section.add_item(label, move |_| {
                        job_p.lock().expect("years job lock").pick = Some(id);
                        Ok(())
                    });
                }
                if open > 0 {
                    for (label, d) in
                        [("Accept", Decision::Accepted), ("Reject", Decision::Rejected), ("Undecide", Decision::Undecided)]
                    {
                        let job = Arc::clone(&job_d);
                        section.add_item(format!("{label} {what}"), move |_| {
                            job.lock().expect("years job lock").decide = Some(d);
                            Ok(())
                        });
                    }
                }
                let job = Arc::clone(&job_c);
                section.add_item("Apply accepted…", move |_| {
                    job.lock().expect("years job lock").confirm = Some(Confirm::Apply);
                    Ok(())
                });
                if !applied.is_empty() {
                    let job = Arc::clone(&job_c);
                    let label = format!("Roll back {}…", applied.iter().map(|i| format!("#{i}")).collect::<Vec<_>>().join(", "));
                    section.add_item(label, move |_| {
                        job.lock().expect("years job lock").confirm = Some(Confirm::Rollback(applied));
                        Ok(())
                    });
                }
                let job = Arc::clone(&job_c);
                section.add_item("Roll back every file's newest apply…", move |_| {
                    job.lock().expect("years job lock").confirm = Some(Confirm::Rollback(Vec::new()));
                    Ok(())
                });
                Some(section)
            })
            .list_section(ctx, move |mut section| {
                let pick = |section: &mut crate::ui::modals::menu::list_section::ListSection, label: String, f: Filter| {
                    let job = Arc::clone(&job_f);
                    section.add_item(label, move |_| {
                        job.lock().expect("years job lock").filter = Some(f);
                        Ok(())
                    });
                };
                for d in DecisionFilter::ORDER {
                    let mark = if filter.decision == d { "●" } else { " " };
                    pick(&mut section, format!("{mark} Show {}", d.label()), Filter { decision: d, ..filter.clone() });
                }
                let mark = |on: bool| if on { "●" } else { " " };
                // the other side's classes and confidences differ: switching sides clears those filters
                pick(
                    &mut section,
                    format!("{} No recording MBID ({no_recording})", mark(filter.no_recording)),
                    Filter { no_recording: !filter.no_recording, confidence: None, class: None, ..filter.clone() },
                );
                pick(
                    &mut section,
                    format!("{} Any confidence", mark(filter.confidence.is_none())),
                    Filter { confidence: None, ..filter.clone() },
                );
                for (c, count) in &confidences {
                    let on = filter.confidence.as_deref() == Some(c.as_str());
                    pick(
                        &mut section,
                        format!("{} Confidence {c} ({count})", mark(on)),
                        Filter { confidence: Some(c.clone()), ..filter.clone() },
                    );
                }
                pick(&mut section, format!("{} Any class", mark(filter.class.is_none())), Filter { class: None, ..filter.clone() });
                for (c, count) in &classes {
                    let on = filter.class.as_deref() == Some(c.as_str());
                    pick(
                        &mut section,
                        format!("{} Class {c} ({count})", mark(on)),
                        Filter { class: Some(c.clone()), ..filter.clone() },
                    );
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }

    fn details(&self, ctx: &Ctx) -> Vec<Line<'static>> {
        let Some(r) = self.selected() else {
            return vec![Line::from("No row in this filter.")];
        };
        let key = ctx.config.theme.preview_label_style;
        let dim = Style::default().add_modifier(Modifier::DIM);
        let field = |name: &str, value: String| Line::from(vec![Span::styled(format!("{name}: "), key), Span::raw(value)]);
        let opt = |v: Option<&String>| v.cloned().unwrap_or_else(|| "–".to_owned());
        let mut decision = r.decision.clone();
        if let Some(at) = &r.decided {
            let _ = write!(decision, " ({})", at.replace('T', " "));
        }
        let mut lines = vec![
            Line::from(Span::styled(r.file.clone(), Style::default().add_modifier(Modifier::BOLD))),
            Line::default(),
            field("Row", format!("#{}", r.id)),
            field("Decision", decision),
        ];
        if let Some(at) = &r.applied {
            lines.push(field("Applied", at.replace('T', " ")));
        }
        lines.push(field("Current TDRC / TDOR", dates_cell(Some(&r.current))));
        lines.push(field("Proposed TDRC / TDOR", dates_cell(r.proposed.as_ref())));
        if let Some(p) = &r.proposed {
            lines.push(field("Date source", opt(p.source.as_ref())));
        }
        lines.push(field("Rule", opt(r.rule.as_ref())));
        lines.push(field("Confidence", opt(r.confidence.as_ref())));
        if !r.classes.is_empty() {
            lines.push(field("Classes", r.classes.join(", ")));
        }
        if let Some(m) = r.mbid_override.as_ref().or(r.mbid.as_ref()) {
            let by_hand = if r.mbid_override.is_some() { " (picked by hand)" } else { "" };
            lines.push(field("Recording", format!("{m}{by_hand}")));
        }
        if let Some((old, pick)) = r.rewrites_recording() {
            let what = if r.applied.is_some() { "Applying wrote" } else { "Applying writes" };
            lines.push(field(
                "Recording tag",
                format!("{what} {pick} over {} (UFID / MUSICBRAINZ_TRACKID; Roll back… restores it)", old.unwrap_or("none")),
            ));
        }
        if let Some(d) = &r.detail {
            if let Some(rel) = &d.release {
                lines.push(field(
                    "Release",
                    format!(
                        "{} \"{}\" {}",
                        rel.kind.as_deref().unwrap_or("?"),
                        rel.title.as_deref().unwrap_or("?"),
                        rel.date.as_deref().unwrap_or("")
                    ),
                ));
            }
            if let Some(rg) = &d.rg_date {
                lines.push(field("Release group", rg.clone()));
            }
            if let Some(own) = &d.tdrc_own {
                lines.push(field("Recording's first release", own.clone()));
            }
            if d.source.is_some() && d.source != d.matched {
                lines.push(field("Source recording", opt(d.source.as_ref())));
            }
        }
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("Evidence", key)));
        for part in r.evidence.split("; ").filter(|p| !p.is_empty()) {
            lines.push(Line::from(format!("· {part}")));
        }
        lines.push(Line::default());
        let hint = match r.locked() {
            Some(why) => why.to_owned(),
            None if r.proposed.is_some() => "a accept · D reject · again: undecide · Enter menu".to_owned(),
            None if r.acceptable() => {
                "the year holds: a accept writes the picked recording tag · D reject · Enter menu".to_owned()
            }
            None => "no proposal: D reject keeps the year · Enter menu: pick the recording".to_owned(),
        };
        lines.push(Line::from(Span::styled(hint, dim)));
        lines
    }
}

/// Run `musicdb` in the background: report its outcome (`describe` turns the output into the message; with
/// `busy` set the outcome opens as a dialog), then reload the report.
fn run_then_reload(
    ctx: &Ctx,
    job: &Arc<Mutex<Job>>,
    args: Vec<String>,
    busy: Option<&'static str>,
    describe: impl FnOnce(&str) -> String + Send + 'static,
) {
    {
        let mut j = job.lock().expect("years job lock");
        if let Some(running) = j.busy {
            status_info!("Years: still {running}, try again when it is done");
            return;
        }
        j.busy = Some(busy.unwrap_or("saving"));
    }
    if let Some(what) = busy {
        status_info!("Years: {what}…");
    }
    let (job, sender) = (Arc::clone(job), ctx.app_event_sender.clone());
    std::thread::spawn(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = deleted::run(&refs);
        let mut j = job.lock().expect("years job lock");
        j.busy = None;
        j.reload = true;
        match result {
            Ok(out) => {
                let text = describe(&out);
                if busy.is_some() {
                    j.result = Some(text);
                } else {
                    status_info!("{text}");
                }
            }
            Err(err) => status_error!("musicdb: {err}"),
        }
        drop(j);
        let _ = sender.send(AppEvent::RequestRender);
    });
}

fn decision_mark(r: &YearRow) -> &'static str {
    match (r.applied.is_some(), r.decision.as_str()) {
        (true, _) => "✓✓",
        (_, "accepted") => "✓",
        (_, "rejected") => "✗",
        _ => "",
    }
}

impl Pane for YearsPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let (loading, error, busy) = self.refresh(ctx);
        // the details beside the table on a wide panel, under it on a narrow one (the table needs the width)
        let [main, details] = if area.width >= WIDE {
            Layout::horizontal([Constraint::Min(50), Constraint::Percentage(35)]).spacing(2).areas(area)
        } else {
            Layout::vertical([Constraint::Min(6), Constraint::Percentage(40)]).spacing(1).areas(area)
        };
        let [top, table_area, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(main);
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.shown.len(), table_area.height.saturating_sub(1).into());

        let dim = Style::default().add_modifier(Modifier::DIM);
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let head = match (&error, loading) {
            (Some(err), _) => Span::styled(format!(" musicdb: {err}"), bold),
            (None, true) if self.report.generated.is_none() => Span::styled(" reading the years report…", dim),
            (None, _) if self.report.generated.is_none() => Span::styled(
                " No report yet: run `musicdb years --dry-run` (about an hour for the library, cached)",
                bold,
            ),
            _ => Span::styled(format!(" {}", summary(&self.report.rows)), ctx.config.theme.preview_label_style),
        };
        frame.render_widget(Paragraph::new(Line::from(head)), top);

        let rows = self.shown.iter().map(|&i| {
            let r = &self.report.rows[i];
            let mark = if self.marked.contains(&r.id) { "▌" } else { "" };
            Row::new(vec![
                Cell::from(mark),
                Cell::from(decision_mark(r)),
                Cell::from(r.id.to_string()),
                Cell::from(r.file.rsplit('/').next().unwrap_or(&r.file).to_owned()),
                Cell::from(dates_cell(Some(&r.current))),
                Cell::from(dates_cell(r.proposed.as_ref())),
                Cell::from(r.rule.clone().unwrap_or_default()),
                Cell::from(r.confidence.clone().unwrap_or_default()),
                Cell::from(r.short_evidence()),
            ])
            .style(if r.locked().is_some() { dim } else { Style::default() })
        });
        let header = Row::new(["", "", "#", "File", "Current", "Proposed", "Rule", "Conf.", "Evidence"])
            .style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(4),
            Constraint::Fill(2),
            Constraint::Length(11),
            Constraint::Length(17),
            Constraint::Length(11),
            Constraint::Length(6),
            Constraint::Fill(3),
        ])
        .header(header)
        .column_spacing(1)
        .style(ctx.config.as_text_style())
        .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let mut status = format!(" showing {} of {} · {}", self.shown.len(), self.report.rows.len(), self.filter.describe());
        if !self.marked.is_empty() {
            let _ = write!(status, " · {} marked", self.marked.len());
        }
        if let Some(what) = busy {
            let _ = write!(status, " · {what}…");
        }
        if let Some(at) = &self.report.generated {
            let _ = write!(status, " · report {}", at.get(..16).unwrap_or(at).replace('T', " "));
        }
        frame.render_widget(Paragraph::new(Line::from(Span::styled(status, dim))), footer);
        frame.render_widget(Paragraph::new(self.details(ctx)).wrap(Wrap { trim: false }), details);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.load(ctx);
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        // an apply or rollback updates MPD's database; a dry run elsewhere rewrites the report
        if matches!(event, UiEvent::Database | UiEvent::Reconnected) && is_visible {
            self.load(ctx);
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if !self.table_area.contains(event.into()) {
            return Ok(());
        }
        let row = usize::from(event.y.saturating_sub(self.table_area.y + 1)); // +1: header row
        match event.kind {
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick => {
                if let Some(idx) = self.state.get_at_rendered_row(row) {
                    self.state.select(Some(idx), ctx.config.scrolloff);
                    if matches!(event.kind, MouseEventKind::DoubleClick) {
                        self.open_menu(ctx);
                    }
                }
            }
            MouseEventKind::ScrollUp => self.state.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff),
            MouseEventKind::ScrollDown => self.state.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff),
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        match action {
            CommonAction::Down => self.state.next(scrolloff, wrap),
            CommonAction::Up => self.state.prev(scrolloff, wrap),
            CommonAction::DownHalf => self.state.next_half_viewport(scrolloff),
            CommonAction::UpHalf => self.state.prev_half_viewport(scrolloff),
            CommonAction::PageDown => self.state.next_viewport(scrolloff),
            CommonAction::PageUp => self.state.prev_viewport(scrolloff),
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::Left | CommonAction::Right => {
                let decision = self.filter.decision.step(matches!(action, CommonAction::Right));
                self.filter.decision = decision;
                self.refilter();
            }
            CommonAction::Select => {
                if let Some(id) = self.selected().map(|r| r.id)
                    && !self.marked.remove(&id)
                {
                    self.marked.insert(id);
                }
                self.state.next(scrolloff, false);
            }
            CommonAction::InvertSelection => {
                let ids: BTreeSet<u64> = self.shown.iter().map(|&i| self.report.rows[i].id).collect();
                self.marked = ids.symmetric_difference(&self.marked).copied().collect();
            }
            CommonAction::AddOptions { .. } => self.decide(Decision::Accepted, ctx),
            CommonAction::Delete => self.decide(Decision::Rejected, ctx),
            CommonAction::Close if self.has_marks() => self.marked.clear(),
            CommonAction::Confirm | CommonAction::ContextMenu => self.open_menu(ctx),
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        ctx.render()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use rmpc_mpd::commands::{Song, metadata_tag::MetadataTag};
    use rstest::rstest;

    use super::*;
    use crate::{
        config::theme::{
            TagResolutionStrategy,
            properties::{Property, PropertyKindOrText, SongProperty, Transform},
        },
        tests::fixtures::ctx,
        ui::song_ext::SongExt as _,
    };

    /// A report as `musicdb years --json` prints it (rormpc-tools years.py `build_row`, `save_report`).
    const REPORT: &str = r#"{"version": 1, "generated": "2026-10-11T00:40:12", "next_id": 4, "rows": [
        {"id": 1, "file": "yt/Artist - Song.mp3", "md5": "m", "mbid": "bf8373da-0000-0000-0000-000000000000",
         "current": {"TDRC": "2000", "TDOR": "2000", "DATE_SOURCE": null, "DATE_RULE": null},
         "proposed": {"TDRC": "2000", "TDOR": "1983-01-04", "DATE_SOURCE": "musicbrainz:aaaa", "DATE_RULE": "same-length"},
         "rule": "same-length", "confidence": "high", "classes": ["video"],
         "evidence": "matched `bf8373da` \"Song\" (video); source `aaaa` \"Song\" on Single \"Song\" 1983-01-04",
         "detail": {"matched": "bf8373da", "source": "aaaa", "release": {"id": "r", "title": "Song", "date": "1983-01-04",
                    "type": "Single"}, "rg_date": "1983-01-04", "tdrc_own": "2000"},
         "needs_mbid": false, "song_id": "s1", "decision": "undecided", "decided": null, "applied": null},
        {"id": 2, "file": "rip/b.flac", "md5": "m", "mbid": null,
         "current": {"TDRC": "1999", "TDOR": null, "DATE_SOURCE": null, "DATE_RULE": null},
         "proposed": null, "rule": null, "confidence": null, "classes": ["no-recording"],
         "evidence": "no MusicBrainz recording in the tags", "detail": null, "needs_mbid": true,
         "decision": "undecided", "decided": null, "applied": null},
        {"id": 3, "file": "yt/c.mp3", "md5": "m", "mbid": "cccc",
         "current": {"TDRC": "1993", "TDOR": "1993", "DATE_SOURCE": null, "DATE_RULE": null},
         "proposed": {"TDRC": "1993", "TDOR": "1971", "DATE_SOURCE": "musicbrainz:dddd", "DATE_RULE": "any-clean"},
         "rule": "any-clean", "confidence": "review", "classes": ["version", "rg-earlier"], "evidence": "e",
         "detail": null, "needs_mbid": false, "decision": "accepted", "decided": "2026-10-11T01:00:00",
         "applied": "2026-10-11T01:05:00"}
    ], "counts": {"rows": 3}}"#;

    fn report() -> Report {
        parse_report(REPORT).expect("a report")
    }

    #[test]
    fn the_report_is_parsed_with_its_rows() {
        let rep = report();
        assert_eq!(rep.rows.len(), 3);
        let r = &rep.rows[0];
        assert_eq!(r.current.tdor.as_deref(), Some("2000"));
        assert_eq!(r.proposed.as_ref().and_then(|p| p.tdor.as_deref()), Some("1983-01-04"));
        assert_eq!(dates_cell(r.proposed.as_ref()), "2000 / 1983-01-04");
        assert_eq!(r.short_evidence(), "video · Single \"Song\"");
        assert_eq!(r.locked(), None);
        assert!(rep.rows[1].locked().is_some_and(|w| w.contains("Pick the recording")));
        assert!(rep.rows[1].wants_pick() && !rep.rows[1].acceptable());
        assert_eq!(rep.rows[1].short_evidence(), "needs MBID · a: pick · no-recording");
        assert_eq!(dates_cell(rep.rows[1].proposed.as_ref()), "–");
        assert!(rep.rows[2].locked().is_some_and(|w| w.contains("Roll back")));
        assert_eq!(
            summary(&rep.rows),
            "3 rows · 2 undecided · 1 accepted · 0 rejected · 1 applied · 1 high · 1 review · 1 need an MBID · 0 picked \
             · 1 without a recording MBID"
        );
    }

    /// Row 2 after `musicdb years --mbid 2 <MBID>` (rormpc-tools 0.2.54 `set_mbid`): computed from the pick, with the
    /// file's recording tag the apply checks and rewrites (`file_mbid`, null: none).
    const PICKED: &str = r#"{"file": "rip/b.flac", "md5": "m", "mbid": "0f5e2d4c-1111-2222-3333-444455556666",
        "current": {"TDRC": "1999", "TDOR": null, "DATE_SOURCE": null, "DATE_RULE": null}, "proposed": null,
        "rule": null, "confidence": null, "classes": ["keeps-year"], "evidence": "the picked recording gives the same year",
        "detail": null, "needs_mbid": false, "id": 2, "song_id": null,
        "mbid_override": "0f5e2d4c-1111-2222-3333-444455556666", "decision": "undecided", "decided": null,
        "applied": null, "file_mbid": null}"#;

    #[test]
    fn a_picked_row_is_open_and_applying_it_rewrites_the_recording_tag() {
        let r: YearRow = serde_json::from_str(PICKED).expect("a row");
        assert_eq!(r.locked(), None);
        assert!(!r.wants_pick() && r.acceptable());
        assert_eq!(r.rewrites_recording(), Some((None, "0f5e2d4c-1111-2222-3333-444455556666")));
        assert_eq!(r.short_evidence(), "picked 0f5e2d4c · keeps-year");
        // a pick MusicBrainz does not know cannot be accepted (rormpc-tools `acceptable`)
        let unknown = YearRow { classes: vec!["recording-not-found".to_owned()], needs_mbid: true, ..r.clone() };
        assert!(!unknown.acceptable() && unknown.locked().is_none());
        // the pick is already the file's tag: nothing to rewrite
        let same = YearRow { file_mbid: r.mbid_override.clone(), ..r.clone() };
        assert_eq!(same.rewrites_recording(), None);
        assert_eq!(
            picked_message(2, "0f5e2d4c-1111-2222-3333-444455556666", PICKED),
            "Years: #2 picked 0f5e2d4c · no new year · undecided: a accepts (applying writes the recording tag)"
        );
        assert_eq!(picked_message(2, "0f5e2d4c-1111", "boom"), "Years: #2 picked 0f5e2d4c");
    }

    #[rstest]
    #[case("0F5E2D4C-1111-2222-3333-444455556666", Ok("0f5e2d4c-1111-2222-3333-444455556666"))]
    #[case("  https://musicbrainz.org/recording/0f5e2d4c-1111-2222-3333-444455556666  ", Ok("0f5e2d4c-1111-2222-3333-444455556666"))]
    #[case("musicbrainz.org/recording/0f5e2d4c-1111-2222-3333-444455556666/edits?x=1", Ok("0f5e2d4c-1111-2222-3333-444455556666"))]
    #[case("https://beta.musicbrainz.org/recording/0f5e2d4c-1111-2222-3333-444455556666#x", Ok("0f5e2d4c-1111-2222-3333-444455556666"))]
    #[case("https://musicbrainz.org/release/0f5e2d4c-1111-2222-3333-444455556666", Err("a release URL"))]
    #[case("https://musicbrainz.org/recording/0f5e2d4c", Err("no recording MBID"))]
    #[case("0f5e2d4c-1111-2222-3333-44445555666", Err("not a MusicBrainz recording URL or MBID"))]
    #[case("0f5e2d4c_1111-2222-3333-444455556666", Err("not a MusicBrainz recording URL or MBID"))]
    #[case("https://example.com/recording/0f5e2d4c-1111-2222-3333-444455556666", Err("not a MusicBrainz"))]
    #[case("", Err("not a MusicBrainz"))]
    fn the_picker_takes_a_recording_url_or_mbid(#[case] input: &str, #[case] want: Result<&str, &str>) {
        match (parse_mbid(input), want) {
            (Ok(got), Ok(want)) => assert_eq!(got, want),
            (Err(got), Err(want)) => assert!(got.contains(want), "{got}"),
            (got, want) => panic!("{input:?}: {got:?}, want {want:?}"),
        }
    }

    #[test]
    fn another_report_version_or_no_years_command_is_said_plainly() {
        let err = parse_report(r#"{"version": 2, "rows": []}"#).expect_err("version 2");
        assert!(err.contains("version 2 (this rormpc reads 1)"));
        // an empty report (no dry run yet) has no rows and no generated time
        let empty = parse_report(r#"{"version": 1, "generated": null, "next_id": 1, "rows": []}"#).expect("empty");
        assert!(empty.generated.is_none() && empty.rows.is_empty());
        let old = "musicdb: error: argument cmd: invalid choice: 'years' (choose from 'sync', 'update')";
        assert!(load_error(old.to_owned()).contains("no `years` command"));
        assert_eq!(load_error("boom".to_owned()), "boom");
    }

    #[test]
    fn the_keys_run_these_musicdb_commands() {
        assert_eq!(decide_args(Decision::Accepted, &[3, 7, 12]), ["years", "--accept", "3", "7", "12"]);
        assert_eq!(decide_args(Decision::Rejected, &[5]), ["years", "--reject", "5"]);
        assert_eq!(decide_args(Decision::Undecided, &[4]), ["years", "--undecide", "4"]);
        assert_eq!(rollback_args(&[]), ["years", "--rollback"]);
        assert_eq!(rollback_args(&[3]), ["years", "--rollback", "3"]);
        assert_eq!(LOAD_ARGS, ["years", "--json"]);
        assert_eq!(APPLY_ARGS, ["years", "--apply"]);
        assert_eq!(mbid_args(2, "0f5e2d4c-1111-2222-3333-444455556666"), [
            "years",
            "--mbid",
            "2",
            "0f5e2d4c-1111-2222-3333-444455556666"
        ]);
    }

    #[test]
    fn the_same_key_again_undecides() {
        let rep = report();
        let (undecided, accepted) = (&rep.rows[0], &rep.rows[2]);
        assert_eq!(toggled(Decision::Accepted, &[undecided]), Decision::Accepted);
        assert_eq!(toggled(Decision::Accepted, &[accepted]), Decision::Undecided);
        // mixed marked rows take the key's decision
        assert_eq!(toggled(Decision::Accepted, &[undecided, accepted]), Decision::Accepted);
        assert_eq!(toggled(Decision::Rejected, &[accepted]), Decision::Rejected);
    }

    #[test]
    fn filters_by_decision_confidence_and_class() {
        let rep = report();
        let ids = |f: &Filter| rep.rows.iter().filter(|r| f.matches(r)).map(|r| r.id).collect::<Vec<_>>();
        // undecided first; #2 has no recording MBID: only under its own filter, off by default
        assert_eq!(ids(&Filter::default()), [1]);
        assert_eq!(ids(&Filter { decision: DecisionFilter::Accepted, ..Filter::default() }), [3]);
        assert_eq!(ids(&Filter { decision: DecisionFilter::All, ..Filter::default() }), [1, 3]);
        let none = Filter { decision: DecisionFilter::All, confidence: Some("none".to_owned()), ..Filter::default() };
        assert!(ids(&none).is_empty());
        let class = Filter { decision: DecisionFilter::All, class: Some("rg-earlier".to_owned()), ..Filter::default() };
        assert_eq!(ids(&class), [3]);
        let no_recording = Filter { no_recording: true, ..Filter::default() };
        assert_eq!(ids(&no_recording), [2]);
        assert_eq!(ids(&Filter { decision: DecisionFilter::Accepted, ..no_recording.clone() }), Vec::<u64>::new());
        assert_eq!(ids(&Filter { confidence: Some("none".to_owned()), ..no_recording.clone() }), [2]);
        assert_eq!(no_recording.describe(), "undecided · no recording MBID");
        assert_eq!(DecisionFilter::Undecided.step(true), DecisionFilter::Accepted);
        assert_eq!(DecisionFilter::Undecided.step(false), DecisionFilter::All);
    }

    #[test]
    fn the_menu_counts_only_the_side_of_the_no_recording_filter_shown() {
        let mut rows = report().rows;
        // a second file without a recording MBID, and #2 after a pick: it joins the other rows
        rows.push(YearRow { id: 4, classes: vec!["no-recording".to_owned(), "no-work".to_owned()], ..rows[1].clone() });
        let counts = |m: &BTreeMap<String, usize>| m.iter().map(|(k, v)| (k.clone(), *v)).collect::<Vec<_>>();
        let pair = |k: &str, v: usize| (k.to_owned(), v);
        let (classes, confidences, no_recording) = filter_counts(&rows, false);
        assert_eq!(counts(&classes), [pair("rg-earlier", 1), pair("version", 1), pair("video", 1)]);
        assert_eq!(counts(&confidences), [pair("high", 1), pair("review", 1)]);
        assert_eq!(no_recording, 2);
        let (classes, confidences, no_recording) = filter_counts(&rows, true);
        assert_eq!(counts(&classes), [pair("no-work", 1)]); // the class itself is the filter, not an entry
        assert_eq!(counts(&confidences), [pair("none", 2)]);
        assert_eq!(no_recording, 2);
        let picked: YearRow = serde_json::from_str(PICKED).expect("a row");
        rows[1] = picked;
        let (classes, confidences, no_recording) = filter_counts(&rows, false);
        assert_eq!(classes.get("keeps-year"), Some(&1));
        assert_eq!(confidences.get("none"), Some(&1));
        assert_eq!(no_recording, 1);
    }

    #[test]
    fn marks_stay_only_on_the_rows_the_filter_shows() {
        let mut pane = YearsPane::new();
        pane.report = report();
        pane.refilter();
        pane.marked = BTreeSet::from([1, 3]);
        pane.filter.decision = DecisionFilter::All;
        pane.refilter();
        assert_eq!(pane.marked, BTreeSet::from([1, 3]));
        pane.filter.decision = DecisionFilter::Accepted;
        pane.refilter();
        assert_eq!(pane.marked, BTreeSet::from([3])); // #1 is undecided: hidden, so no longer marked
        assert_eq!(pane.targets().iter().map(|r| r.id).collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn apply_and_rollback_results_become_the_dialog() {
        let out = r#"{"applied": [{"id": 1, "file": "yt/a.mp3", "new": {}}], "skipped": [
            {"id": 3, "file": "yt/c.mp3", "why": "audio changed since the report"}]}"#;
        assert_eq!(
            outcome_message("Applied", out),
            "Applied 1 row.\n  #1 yt/a.mp3\n\nSkipped 1 row:\n  #3 yt/c.mp3: audio changed since the report"
        );
        let back = r#"{"rolled_back": [{"id": 1, "file": "a.mp3", "restored": {}}, {"id": 2, "file": "b.mp3",
            "restored": {}}], "skipped": []}"#;
        assert_eq!(outcome_message("Rolled back", back), "Rolled back 2 rows.\n  #1 a.mp3\n  #2 b.mp3");
        assert_eq!(outcome_message("Applied", "not json"), "Applied: not json");
        // an applied pick (rormpc-tools 0.2.54): the recording tag it rewrote
        let pick = r#"{"applied": [{"id": 2, "file": "rip/b.flac", "new": {}, "recording": {"old": null,
            "new": "0f5e2d4c-1111-2222-3333-444455556666", "rekeyed": {}}}], "skipped": []}"#;
        assert_eq!(outcome_message("Applied", pick), "Applied 1 row.\n  #2 rip/b.flac · recording tag none → 0f5e2d4c");
    }

    fn song(tags: &[(&str, &str)]) -> Song {
        Song {
            file: "a.mp3".to_owned(),
            metadata: tags.iter().map(|(k, v)| ((*k).to_owned(), MetadataTag::Single((*v).to_owned()))).collect::<HashMap<_, _>>(),
            ..Default::default()
        }
    }

    #[test]
    fn the_shown_date_is_originaldate_falling_back_to_date() {
        assert_eq!(shown_date(&song(&[("date", "2000"), ("originaldate", "1983-01-04")])), "1983-01-04");
        assert_eq!(shown_date(&song(&[("date", "2000")])), "2000");
        assert_eq!(shown_date(&song(&[])), "");
    }

    /// The theme's Year column (dotfiles roman.ron): originaldate's year, else date's.
    #[rstest]
    #[case(&[("date", "2000"), ("originaldate", "1983-01-04")], "1983")]
    #[case(&[("date", "1999-05-01")], "1999")]
    fn the_year_column_reads_originaldate_falling_back_to_date(
        #[case] tags: &[(&str, &str)],
        #[case] year: &str,
        ctx: Ctx,
    ) {
        let other = |tag: &str| Property { kind: PropertyKindOrText::Property(SongProperty::Other(tag.to_owned())), style: None, default: None };
        let format = Property::<SongProperty> {
            kind: PropertyKindOrText::Transform(Transform::Truncate {
                content: Box::new(Property { default: Some(Box::new(other("date"))), ..other("originaldate") }),
                length: 4,
                from_start: false,
            }),
            style: None,
            default: None,
        };
        let song = song(tags);
        let line = song.as_line(&format, "", TagResolutionStrategy::All, &ctx).expect("a year");
        assert_eq!(line.to_string(), year);
    }
}
