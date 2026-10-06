//! rormpc: Shuffle pane. What plays next with mpd-player's weighted shuffle, in play order: the song playing now,
//! the Up next requests (they always come first), then the shuffle's plan (the next songs it drew ahead, with why).
//! The plan changes only when a planned song leaves the queue, is requested, gets "heard enough" or is played by
//! hand, and is topped up after every song. A view only: nothing here reorders the queue. Enter on a planned song
//! asks for it with Play next, on a request plays it now; C (JumpToCurrent) goes to the playing song.

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState},
};

use super::Pane;
use crate::{
    config::keys::{CommonAction, QueueActions},
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        macros::modal,
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{dirstack::DirState, modals::menu::modal::MenuModal, rormpc_player, rormpc_upnext},
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    Now,
    /// an Up next request, by queue id
    Request(u32),
    Planned,
}

#[derive(Debug, Clone)]
struct Item {
    kind: Kind,
    file: String,
    why: String,
}

#[derive(Debug)]
pub struct ShufflePane {
    items: Vec<Item>,
    state: DirState<TableState>,
    table_area: Rect,
}

/// (title, artist) of a queued file, else its file name.
fn name(ctx: &Ctx, file: &str) -> (String, String) {
    let song = ctx.queue.iter().find(|s| s.file == file);
    let tag = |t: &str| song.and_then(|s| s.metadata.get(t)).map(|v| v.last().to_owned()).unwrap_or_default();
    let title = tag("title");
    (if title.is_empty() { file.rsplit('/').next().unwrap_or(file).to_owned() } else { title }, tag("artist"))
}

impl ShufflePane {
    pub fn new() -> Self {
        Self { items: Vec::new(), state: DirState::default(), table_area: Rect::default() }
    }

    fn selected(&self) -> Option<&Item> {
        self.state.get_selected().and_then(|i| self.items.get(i))
    }

    fn confirm(&self, ctx: &Ctx) {
        match self.selected().map(|i| (i.kind.clone(), i.file.clone())) {
            Some((Kind::Planned, file)) => rormpc_upnext::play_next(ctx, vec![file]),
            Some((Kind::Request(id), _)) => rormpc_upnext::play_entry(ctx, id),
            _ => {}
        }
    }

    fn open_menu(&self, ctx: &Ctx) {
        let Some(item) = self.selected().cloned() else { return };
        let title = name(ctx, &item.file).0;
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
                if item.kind == Kind::Planned {
                    let f = item.file.clone();
                    section.add_item("Play next (Up next)", move |ctx| {
                        rormpc_upnext::play_next(ctx, vec![f.clone()]);
                        Ok(())
                    });
                }
                let f = item.file.clone();
                section.add_item("Heard enough (rests in the weighted shuffle)", move |ctx| {
                    rormpc_player::heard_enough(ctx, f.clone(), title.clone());
                    Ok(())
                });
                Some(section)
            })
            .list_section(ctx, |section| Some(section.item("Cancel", |_| Ok(()))))
            .build();
        modal!(ctx, menu);
    }
}

impl Pane for ShufflePane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let sh = rormpc_player::shuffle_state();
        let dim = Style::default().add_modifier(Modifier::DIM);
        let key = ctx.config.theme.preview_label_style;

        // play order: now, the requests, the plan
        let keep = self.selected().map(|i| (i.kind.clone(), i.file.clone()));
        let mut items = Vec::new();
        if let Some(cur) = ctx.current_song() {
            items.push(Item { kind: Kind::Now, file: cur.file.clone(), why: "playing now".to_owned() });
        }
        for w in rormpc_upnext::waiting(ctx) {
            items.push(Item { kind: Kind::Request(w.id), file: w.file, why: "Up next request (plays first)".to_owned() });
        }
        if sh.enabled && sh.active {
            items.extend(sh.plan.iter().map(|p| Item { kind: Kind::Planned, file: p.file.clone(), why: p.why.clone() }));
        }
        self.items = items;

        let status = match (sh.enabled, sh.active) {
            (false, _) => "Weighted shuffle is off (w turns it on): the queue plays in order, or MPD's random with x.".to_owned(),
            (true, false) => format!("Weighted shuffle waiting: {}", sh.reason),
            (true, true) => format!(
                "In play order: the song playing, the Up next requests, then {} songs the shuffle drew ahead.",
                sh.plan.len()
            ),
        };
        let [head, table_area, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)]).areas(area);
        frame.render_widget(Paragraph::new(Line::from(Span::styled(status, dim))), head);

        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.items.len(), table_area.height.saturating_sub(1).into());
        let idx = keep
            .and_then(|(k, f)| self.items.iter().position(|i| i.kind == k && i.file == f))
            .or_else(|| self.state.get_selected().map(|i| i.min(self.items.len().saturating_sub(1))))
            .unwrap_or(0);
        self.state.select((!self.items.is_empty()).then_some(idx), ctx.config.scrolloff);

        let mut n = 0;
        let rows = self.items.iter().map(|i| {
            let (title, artist) = name(ctx, &i.file);
            let mark = match i.kind {
                Kind::Now => "▶".to_owned(),
                Kind::Request(_) => "↑".to_owned(),
                Kind::Planned => {
                    n += 1;
                    n.to_string()
                }
            };
            let row = Row::new(vec![
                Cell::from(mark),
                Cell::from(title),
                Cell::from(artist),
                Cell::from(Span::styled(i.why.clone(), dim)),
            ]);
            if i.kind == Kind::Now { row.style(Style::default().add_modifier(Modifier::BOLD)) } else { row }
        });
        let header = Row::new(["", "Title", "Artist", "Why"]).style(key);
        let table = Table::new(rows, [
            Constraint::Length(3),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Min(20),
        ])
        .header(header)
        .column_spacing(1)
        .style(ctx.config.as_text_style())
        .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let foot = " the plan changes only when a planned song is requested, rested or played by hand · Enter: Play next · C: playing song · Ctrl-z: heard enough";
        frame.render_widget(Paragraph::new(Line::from(Span::styled(foot, dim))), footer);
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
                        self.confirm(ctx);
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
        // the Queue's JumpToCurrent (C) works here too: the playing song is the first row
        if event.actions.iter().any(|a| matches!(a.as_queue(), Some(QueueActions::JumpToCurrent))) {
            event.claim_queue();
            self.state.first();
            ctx.render()?;
            return Ok(());
        }
        let Some(action) = event.claim_common().cloned() else {
            return Ok(());
        };
        let (scrolloff, wrap) = (ctx.config.scrolloff, ctx.config.wrap_navigation);
        match action {
            CommonAction::Down => self.state.next(scrolloff, wrap),
            CommonAction::Up => self.state.prev(scrolloff, wrap),
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::Confirm => self.confirm(ctx),
            CommonAction::ContextMenu => self.open_menu(ctx),
            _ => {
                event.abandon(); // not ours: let global keys (tabs, playback) handle it
                return Ok(());
            }
        }
        ctx.render()?;
        Ok(())
    }
}
