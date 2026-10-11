//! rormpc: the Banned panel over Music (`gB`, `ShowPlay(Banned)`): the songs a hand removal from the queue banned
//! (`musicdb banned --json`, newest first; `rormpc_banned`). Enter, `u` or a double click unbans the selected song
//! (`musicdb unban FILE`); the list is read again after each change, and Music reruns its filters so the song
//! comes back into the queue when they match it.

use std::sync::{Arc, Mutex};

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
};

use super::{Pane, deleted::run};
use crate::{
    config::keys::{CommonAction, GlobalAction},
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        UiEvent,
        dirstack::DirState,
        rormpc_banned::{self, BannedSong},
    },
};

/// State shared with the background `musicdb` runs.
#[derive(Debug, Default)]
struct Job {
    loading: bool,
    /// a newer list to take on the next render
    rows: Option<Vec<BannedSong>>,
    error: Option<String>,
    /// an unban finished: load again
    reload: bool,
    /// songs unbanned since Music last asked (`take_unbanned`)
    unbanned: Vec<String>,
}

#[derive(Debug)]
pub struct BannedPane {
    rows: Vec<BannedSong>,
    state: DirState<TableState>,
    table_area: Rect,
    job: Arc<Mutex<Job>>,
}

impl BannedPane {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            state: DirState::default(),
            table_area: Rect::default(),
            job: Arc::new(Mutex::new(Job::default())),
        }
    }

    /// Read the list in a background thread.
    fn load(&self, ctx: &Ctx) {
        let mut job = self.job.lock().expect("banned job lock");
        if job.loading {
            return;
        }
        job.loading = true;
        drop(job);
        let (job, sender) = (Arc::clone(&self.job), ctx.app_event_sender.clone());
        std::thread::spawn(move || {
            let result = run(&["banned", "--json"]).and_then(|out| rormpc_banned::parse(&out));
            let mut j = job.lock().expect("banned job lock");
            j.loading = false;
            match result {
                Ok(rows) => {
                    j.rows = Some(rows);
                    j.error = None;
                }
                Err(err) => j.error = Some(err),
            }
            drop(j);
            let _ = sender.send(AppEvent::RequestRender);
        });
    }

    /// Take a finished load (keeping the selected song) and start the reload an unban asked for. Returns whether
    /// a load runs and its error.
    fn refresh(&mut self, ctx: &Ctx) -> (bool, Option<String>) {
        let (fresh, reload, loading, error) = {
            let mut j = self.job.lock().expect("banned job lock");
            (j.rows.take(), std::mem::take(&mut j.reload), j.loading, j.error.clone())
        };
        if reload {
            self.load(ctx);
        }
        if let Some(rows) = fresh {
            let keep = self.state.get_selected();
            let keep_key = keep.and_then(|i| self.rows.get(i)).map(|r| r.key.clone());
            self.rows = rows;
            self.state.set_content_and_viewport_len(self.rows.len(), self.table_area.height.saturating_sub(1).into());
            // the same song, else the row where the unbanned one was
            let idx = keep_key
                .and_then(|k| self.rows.iter().position(|r| r.key == k))
                .or(keep)
                .map_or(0, |i| i.min(self.rows.len().saturating_sub(1)));
            self.state.select((!self.rows.is_empty()).then_some(idx), 0);
        }
        (loading, error)
    }

    /// The files unbanned here since the last call: Music reruns its filters for them.
    pub fn take_unbanned(&self) -> Vec<String> {
        std::mem::take(&mut self.job.lock().expect("banned job lock").unbanned)
    }

    fn selected(&self) -> Option<&BannedSong> {
        self.state.get_selected().and_then(|i| self.rows.get(i))
    }

    /// Enter, `u` or a double click: `musicdb unban FILE` in the background, then the list again.
    fn unban_selected(&self, ctx: &Ctx) {
        let Some(song) = self.selected() else { return };
        let (job, file) = (Arc::clone(&self.job), song.file.clone());
        rormpc_banned::set_banned(ctx, vec![file.clone()], false, move || {
            let mut j = job.lock().expect("banned job lock");
            j.reload = true;
            j.unbanned.push(file);
        });
    }
}

/// "2026-10-11 14:02" from an ISO local time.
fn when(ts: &str) -> String {
    ts.get(..16).unwrap_or(ts).replace('T', " ")
}

impl Pane for BannedPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let (loading, error) = self.refresh(ctx);
        let [table_area, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.rows.len(), table_area.height.saturating_sub(1).into());

        let rows = self.rows.iter().map(|r| {
            let title = if r.title.is_empty() { r.file.clone() } else { r.title.clone() };
            Row::new(vec![Cell::from(when(&r.ts)), Cell::from(r.artist.clone()), Cell::from(title)])
        });
        let header = Row::new(["Banned", "Artist", "Title"]).style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [Constraint::Length(16), Constraint::Percentage(35), Constraint::Min(10)])
            .header(header)
            .column_spacing(1)
            .style(ctx.config.as_text_style())
            .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let dim = Style::default().add_modifier(Modifier::DIM);
        let status = match (loading, error) {
            (_, Some(err)) => Span::styled(format!(" musicdb: {err}"), Style::default().add_modifier(Modifier::BOLD)),
            (true, None) if self.rows.is_empty() => Span::styled(" reading the banned songs…", dim),
            _ if self.rows.is_empty() => {
                Span::styled(" Nothing banned: d on a queue song removes and bans it (z undoes)", dim)
            }
            _ => Span::styled(
                format!(" {} banned · never played or filtered in, file kept · Enter or u unbans", self.rows.len()),
                dim,
            ),
        };
        frame.render_widget(Paragraph::new(Line::from(status)), footer);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.load(ctx);
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        if matches!(event, UiEvent::Reconnected) && is_visible {
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
                        self.unban_selected(ctx);
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
        // u (the global Update elsewhere) unbans here, as Enter does
        if event.actions.iter().any(|a| matches!(a.as_global(), Some(GlobalAction::Update))) {
            let _ = event.claim_global();
            self.unban_selected(ctx);
            return Ok(ctx.render()?);
        }
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
            CommonAction::Confirm => self.unban_selected(ctx),
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
    use super::when;

    #[test]
    fn the_ban_time_is_shown_to_the_minute() {
        assert_eq!(when("2026-10-11T14:02:11"), "2026-10-11 14:02");
        assert_eq!(when("2026-10-11"), "2026-10-11");
    }
}
