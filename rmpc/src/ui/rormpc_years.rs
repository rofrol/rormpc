//! rormpc: the "Years to review" panel over Music (`gY`, `ShowPlay(Years)`; rormpc-tools
//! docs/release-years-plan.md, stage B). It reads `musicdb years --json` (the dry-run report, version 1): one row
//! per file whose shown year (`OriginalDate`, else `Date`) would change or that is suspect, with the current and
//! proposed TDRC/TDOR, the rule, the confidence, the evidence and the decision. Every row is reviewed (decided by
//! the user 2026-10-10): `a` accepts, `D` rejects, the same key again on rows already so decided makes them
//! undecided (`musicdb years --accept/--reject/--undecide ID...`); Space marks rows and the keys act on the marked
//! ones. "Apply accepted…" (`musicdb years --apply`) and "Roll back…" (`--rollback [ID...]`) run in the background
//! after a confirmation and show their result; the writing, its checks and the backup stay in rormpc-tools.
//! Rows that need an MBID (no `MusicBrainz` recording or work) are read-only here until the MBID picker comes.
//! h/l switch the decision filter; the menu (Enter) also filters by confidence and class.

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
        macros::{modal, status_error, status_info},
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        UiEvent,
        dirstack::DirState,
        modals::{
            confirm_modal::{Action, ConfirmModal},
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
}

impl YearRow {
    /// Read-only here: rows that need an MBID (the picker comes next) and rows already written (Roll back… first).
    fn locked(&self) -> Option<&'static str> {
        if self.needs_mbid {
            Some("needs an MBID: the MBID picker comes next (meanwhile `musicdb years --mbid ID MBID`)")
        } else if self.applied.is_some() {
            Some("already applied: Roll back… (menu) first")
        } else {
            None
        }
    }

    fn confidence_key(&self) -> &str {
        self.confidence.as_deref().unwrap_or("none")
    }

    /// The table's short evidence: the suspect classes and the source release.
    fn short_evidence(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if self.needs_mbid {
            parts.push("needs MBID · MBID picker comes next".to_owned());
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
    #[derive(Deserialize)]
    struct Done {
        id: u64,
        file: String,
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
}

impl Filter {
    fn matches(&self, r: &YearRow) -> bool {
        (self.decision == DecisionFilter::All || r.decision == self.decision.label())
            && self.confidence.as_ref().is_none_or(|c| r.confidence_key() == c)
            && self.class.as_ref().is_none_or(|c| r.classes.contains(c))
    }

    fn describe(&self) -> String {
        let mut out = self.decision.label().to_owned();
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
        "{} rows · {} undecided · {} accepted · {} rejected · {} applied · {} high · {} review · {} need an MBID",
        rows.len(),
        count(&|r| r.decision == "undecided"),
        count(&|r| r.decision == "accepted"),
        count(&|r| r.decision == "rejected"),
        count(&|r| r.applied.is_some()),
        count(&|r| r.confidence.as_deref() == Some("high")),
        count(&|r| r.confidence.as_deref() == Some("review")),
        count(&|r| r.needs_mbid),
    )
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
        let (fresh, reload, loading, error, busy, result, filter, decide, confirm) = {
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
        let (open, locked): (Vec<&YearRow>, Vec<&YearRow>) = targets.into_iter().partition(|r| r.locked().is_none());
        let decision = if key == Decision::Undecided { key } else { toggled(key, &open) };
        let (ok, no_proposal): (Vec<&YearRow>, Vec<&YearRow>) =
            open.into_iter().partition(|r| decision != Decision::Accepted || r.proposed.is_some());
        let mut notes = Vec::new();
        if let [one] = locked.as_slice() {
            notes.push(format!("#{} {}", one.id, one.locked().unwrap_or_default()));
        } else if !locked.is_empty() {
            notes.push(format!("{} rows left out (need an MBID or already applied)", locked.len()));
        }
        if !no_proposal.is_empty() {
            notes.push(format!("{} rows have no proposal to accept", no_proposal.len()));
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
        let n = self
            .report
            .rows
            .iter()
            .filter(|r| r.decision == "accepted" && r.proposed.is_some() && r.applied.is_none())
            .count();
        if n == 0 {
            status_info!("No accepted rows waiting to be applied");
            return;
        }
        let message = format!(
            "Write the years of {n} accepted row{}?\n\nOnly files whose audio and date tags are what the report saw \
             are written (a changed file is skipped and listed). The old values go to years-backup.jsonl first; \
             Roll back… puts them back. MPD is updated afterwards.",
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
            "Roll back the newest apply of every file?\n\nEach file gets back the date tags the apply replaced, \
             only while it still has what the apply wrote. The rows go back to undecided."
                .to_owned()
        } else {
            format!(
                "Roll back {}?\n\nThe file gets back the date tags the apply replaced, only while it still has what \
                 the apply wrote. The row goes back to undecided.",
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

    fn open_menu(&self, ctx: &Ctx) {
        let targets = self.targets();
        let n = targets.len();
        let open = targets.iter().filter(|r| r.locked().is_none()).count();
        let applied: Vec<u64> = targets.iter().filter(|r| r.applied.is_some()).map(|r| r.id).collect();
        let what = if self.marked.is_empty() { "this row".to_owned() } else { format!("{n} marked rows") };
        let (job_d, job_f, job_c) = (Arc::clone(&self.job), Arc::clone(&self.job), Arc::clone(&self.job));
        let filter = self.filter.clone();
        let mut classes: BTreeMap<String, usize> = BTreeMap::new();
        let mut confidences: BTreeMap<String, usize> = BTreeMap::new();
        for r in &self.report.rows {
            for c in &r.classes {
                *classes.entry(c.clone()).or_default() += 1;
            }
            *confidences.entry(r.confidence_key().to_owned()).or_default() += 1;
        }
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
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
            None => "no proposal: D reject keeps the year · Enter menu".to_owned(),
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
        assert!(rep.rows[1].locked().is_some_and(|w| w.contains("MBID picker comes next")));
        assert_eq!(rep.rows[1].short_evidence(), "needs MBID · MBID picker comes next · no-recording");
        assert_eq!(dates_cell(rep.rows[1].proposed.as_ref()), "–");
        assert!(rep.rows[2].locked().is_some_and(|w| w.contains("Roll back")));
        assert_eq!(
            summary(&rep.rows),
            "3 rows · 2 undecided · 1 accepted · 0 rejected · 1 applied · 1 high · 1 review · 1 need an MBID"
        );
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
        assert_eq!(ids(&Filter::default()), [1, 2]); // undecided first
        assert_eq!(ids(&Filter { decision: DecisionFilter::Accepted, ..Filter::default() }), [3]);
        assert_eq!(ids(&Filter { decision: DecisionFilter::All, ..Filter::default() }), [1, 2, 3]);
        let none = Filter { decision: DecisionFilter::All, confidence: Some("none".to_owned()), class: None };
        assert_eq!(ids(&none), [2]);
        let class = Filter { decision: DecisionFilter::All, confidence: None, class: Some("rg-earlier".to_owned()) };
        assert_eq!(ids(&class), [3]);
        assert_eq!(DecisionFilter::Undecided.step(true), DecisionFilter::Accepted);
        assert_eq!(DecisionFilter::Undecided.step(false), DecisionFilter::All);
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
