//! rormpc: Up next pane. The songs asked for with "Play next", in the order they will play before the rest of the
//! source (the list itself lives in `rormpc_upnext`). Only request actions: play now, move (K/J), make next,
//! remove (D), clear. There is no "Play next" here: asking again elsewhere moves a waiting song to the top.

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
};

use super::Pane;
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        macros::modal,
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        dirstack::DirState,
        modals::menu::modal::MenuModal,
        rormpc_upnext::{self, Waiting},
    },
};

#[derive(Debug)]
pub struct UpNextPane {
    rows: Vec<Waiting>,
    /// the selected entry's queue id, so the selection follows it when the list changes
    selected_id: Option<u32>,
    state: DirState<TableState>,
    table_area: Rect,
}

fn tag(song: Option<&rmpc_mpd::commands::Song>, name: &str) -> String {
    song.and_then(|s| s.metadata.get(name)).map(|v| v.last().to_owned()).unwrap_or_default()
}

fn duration(song: Option<&rmpc_mpd::commands::Song>) -> String {
    song.and_then(|s| s.duration).map_or_else(String::new, |d| {
        let secs = d.as_secs();
        format!("{}:{:02}", secs / 60, secs % 60)
    })
}

impl UpNextPane {
    pub fn new() -> Self {
        Self { rows: Vec::new(), selected_id: None, state: DirState::default(), table_area: Rect::default() }
    }

    fn selected(&self) -> Option<&Waiting> {
        self.state.get_selected().and_then(|i| self.rows.get(i))
    }

    fn select_idx(&mut self, idx: usize, ctx: &Ctx) {
        self.state.select(Some(idx), ctx.config.scrolloff);
        self.selected_id = self.rows.get(idx).map(|w| w.id);
    }

    fn open_menu(&self, ctx: &Ctx) {
        let selected = self.selected().map(|w| w.id);
        let n = self.rows.len();
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
                if let Some(id) = selected {
                    section.add_item("Play now", move |ctx| {
                        rormpc_upnext::play_entry(ctx, id);
                        Ok(())
                    });
                    section.add_item("Make next", move |ctx| {
                        rormpc_upnext::make_next(ctx, id);
                        Ok(())
                    });
                    section.add_item("Remove from Up next (D)", move |ctx| {
                        rormpc_upnext::remove(ctx, id);
                        Ok(())
                    });
                }
                if crate::ui::rormpc_player::shuffle_state().round.is_some_and(|r| r.done) {
                    section.add_item("New round (every song of the source once more)", |ctx| {
                        crate::ui::rormpc_player::new_round(ctx);
                        Ok(())
                    });
                }
                if n > 0 {
                    section.add_item(format!("Clear Up next ({n})…"), move |ctx| {
                        rormpc_upnext::confirm_clear(ctx, n);
                        Ok(())
                    });
                }
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }
}

impl Pane for UpNextPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let (rows, error) = rormpc_upnext::waiting_and_error(ctx);
        self.rows = rows;
        let [table_area, footer] = Layout::vertical([Constraint::Min(1), Constraint::Length(2)]).areas(area);
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.rows.len(), table_area.height.saturating_sub(1).into());
        let idx = self
            .selected_id
            .and_then(|id| self.rows.iter().position(|w| w.id == id))
            .or_else(|| self.state.get_selected().map(|i| i.min(self.rows.len().saturating_sub(1))))
            .or(Some(0));
        self.state.select(idx.filter(|_| !self.rows.is_empty()), ctx.config.scrolloff);
        self.selected_id = self.selected().map(|w| w.id);

        let dim = Style::default().add_modifier(Modifier::DIM);
        let rows = self.rows.iter().enumerate().map(|(i, w)| {
            let song = ctx.queue.iter().find(|s| s.id == w.id);
            let title = tag(song, "title");
            Row::new(vec![
                Cell::from(format!("{}", i + 1)),
                Cell::from(tag(song, "artist")),
                Cell::from(if title.is_empty() { w.file.clone() } else { title }),
                Cell::from(duration(song)),
                Cell::from(Span::styled(if w.added { "added" } else { "source" }, dim)),
            ])
        });
        let header = Row::new(["#", "Artist", "Title", "Time", "From"]).style(ctx.config.theme.preview_label_style);
        let table = Table::new(rows, [
            Constraint::Length(3),
            Constraint::Percentage(30),
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(6),
        ])
        .header(header)
        .column_spacing(1)
        .style(ctx.config.as_text_style())
        .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let hint = if let Some(error) = &error {
            format!(" Up next: {error}")
        } else if self.rows.is_empty() {
            " Nothing waiting. \"Play next\" in a song's menu (Ctrl-z) in any tab puts it here.".to_owned()
        } else {
            format!(
                " {} waiting · Enter play now · K/J move · D remove · Ctrl-z make next, clear",
                self.rows.len()
            )
        };
        // after Up next: the weighted shuffle's pick (mpd-player), which may still change
        let sh = crate::ui::rormpc_player::shuffle_state();
        let then = match (&sh.nominee, sh.enabled, sh.active) {
            (_, false, _) => " Then: MPD's order (weighted shuffle off, w turns it on)".to_owned(),
            (Some(n), _, true) => {
                let song = ctx.queue.iter().find(|s| s.id == n.id);
                let name = match (tag(song, "artist"), tag(song, "title")) {
                    (a, t) if !t.is_empty() => format!("{a} - {t}"),
                    _ => n.file.clone(),
                };
                format!(" Then likely: {name} · shuffle pick, {} · may change", n.why)
            }
            _ if sh.round.as_ref().is_some_and(|r| r.done) => format!(" Then: {} (Ctrl-z: new round)", sh.reason),
            _ => format!(" Then: MPD's order (weighted shuffle waiting: {})", sh.reason),
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(hint, if error.is_some() { Style::default().fg(Color::Red) } else { dim })),
                Line::from(Span::styled(then, dim)),
            ]),
            footer,
        );
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
                    self.select_idx(idx, ctx);
                    if matches!(event.kind, MouseEventKind::DoubleClick)
                        && let Some(id) = self.selected_id
                    {
                        rormpc_upnext::play_entry(ctx, id);
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
        let selected = self.selected_id;
        match action {
            CommonAction::Down => self.state.next(scrolloff, wrap),
            CommonAction::Up => self.state.prev(scrolloff, wrap),
            CommonAction::DownHalf => self.state.next_half_viewport(scrolloff),
            CommonAction::UpHalf => self.state.prev_half_viewport(scrolloff),
            CommonAction::PageDown => self.state.next_viewport(scrolloff),
            CommonAction::PageUp => self.state.prev_viewport(scrolloff),
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::MoveUp => {
                if let Some(id) = selected {
                    rormpc_upnext::move_entry(ctx, id, -1);
                }
            }
            CommonAction::MoveDown => {
                if let Some(id) = selected {
                    rormpc_upnext::move_entry(ctx, id, 1);
                }
            }
            CommonAction::Delete => {
                if let Some(id) = selected {
                    rormpc_upnext::remove(ctx, id);
                }
            }
            CommonAction::Confirm => {
                if let Some(id) = selected {
                    rormpc_upnext::play_entry(ctx, id);
                }
            }
            CommonAction::ContextMenu => self.open_menu(ctx),
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        // the cursor follows the moved entry (MoveUp/MoveDown keep selected_id); other moves pick by index
        if !matches!(action, CommonAction::MoveUp | CommonAction::MoveDown) {
            self.selected_id = self.selected().map(|w| w.id);
        }
        ctx.render()?;
        Ok(())
    }
}
