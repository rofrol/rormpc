//! rormpc: Shuffle pane. What mpd-player's weighted shuffle does next: the song it already picked (with why), and
//! the candidates of the draw after it with their chance in that draw. It is a view only: nothing here reorders
//! the queue. A draw is random and is redrawn after every song, so the list says "candidates", not an order.
//! Enter (or the menu) asks for a candidate with Play next; the menu also has "heard enough".

use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    prelude::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Cell, Paragraph, Row, Table, TableState, Wrap},
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
    ui::{dirstack::DirState, modals::menu::modal::MenuModal, rormpc_player, rormpc_upnext},
};

#[derive(Debug)]
pub struct ShufflePane {
    files: Vec<String>,
    state: DirState<TableState>,
    table_area: Rect,
}

/// "Artist - Title" of a queued file, else its file name.
fn name(ctx: &Ctx, file: &str) -> (String, String) {
    let song = ctx.queue.iter().find(|s| s.file == file);
    let tag = |t: &str| song.and_then(|s| s.metadata.get(t)).map(|v| v.last().to_owned()).unwrap_or_default();
    let title = tag("title");
    (tag("artist"), if title.is_empty() { file.rsplit('/').next().unwrap_or(file).to_owned() } else { title })
}

impl ShufflePane {
    pub fn new() -> Self {
        Self { files: Vec::new(), state: DirState::default(), table_area: Rect::default() }
    }

    fn selected(&self) -> Option<&String> {
        self.state.get_selected().and_then(|i| self.files.get(i))
    }

    fn open_menu(&self, ctx: &Ctx) {
        let Some(file) = self.selected().cloned() else { return };
        let title = name(ctx, &file).1;
        let menu = MenuModal::new(ctx)
            .list_section(ctx, move |mut section| {
                let f = file.clone();
                section.add_item("Play next (Up next)", move |ctx| {
                    rormpc_upnext::play_next(ctx, vec![f.clone()]);
                    Ok(())
                });
                section.add_item("Heard enough (rests in the weighted shuffle)", move |ctx| {
                    rormpc_player::heard_enough(ctx, file.clone(), title.clone());
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
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let key = ctx.config.theme.preview_label_style;

        // what is decided: the pick (Up next requests play before it)
        let mut head: Vec<Line> = Vec::new();
        let up = rormpc_upnext::up_next_ids().len();
        match (&sh.nominee, sh.enabled, sh.active) {
            (_, false, _) => head.push(Line::from(Span::styled(
                "Weighted shuffle is off (w turns it on). With it off the queue plays in order, or MPD's plain random with x.",
                dim,
            ))),
            (Some(n), _, true) => {
                let (artist, title) = name(ctx, &n.file);
                head.push(Line::from(vec![
                    Span::styled("Picked next: ", key),
                    Span::styled(title, bold),
                    Span::raw(if artist.is_empty() { String::new() } else { format!(" - {artist}") }),
                ]));
                head.push(Line::from(Span::styled(format!("  {}", n.why), dim)));
                if up > 0 {
                    head.push(Line::from(Span::styled(format!("  Up next ({up}) plays before it."), dim)));
                }
            }
            _ => head.push(Line::from(Span::styled(format!("Weighted shuffle waiting: {}", sh.reason), dim))),
        }
        let outlook = sh.outlook.clone().filter(|_| sh.enabled && sh.active).unwrap_or_default();
        if !outlook.top.is_empty() {
            let lent = if outlook.drawn_from != outlook.lane && outlook.lane != "next cycle" {
                format!(" (lent to {})", outlook.drawn_from)
            } else {
                String::new()
            };
            head.push(Line::default());
            head.push(Line::from(vec![
                Span::styled("Then a draw: ", key),
                Span::raw(format!(
                    "{} lane{lent} · {} candidates of {} in the queue ({} resting, recent or requested) · redrawn after every song",
                    outlook.lane,
                    outlook.pool,
                    outlook.queued,
                    outlook.queued.saturating_sub(outlook.eligible + 1),
                )),
            ]));
        }
        let head_h = head.len() as u16;
        let [head_area, table_area, footer] =
            Layout::vertical([Constraint::Length(head_h), Constraint::Min(1), Constraint::Length(1)]).areas(area);
        frame.render_widget(Paragraph::new(head).wrap(Wrap { trim: false }), head_area);

        // the candidates of that draw: a chance each, not an order
        self.files = outlook.top.iter().map(|c| c.file.clone()).collect();
        self.table_area = table_area;
        self.state.set_content_and_viewport_len(self.files.len(), table_area.height.saturating_sub(1).into());
        if self.state.get_selected().is_none_or(|i| i >= self.files.len()) {
            self.state.select((!self.files.is_empty()).then_some(0), 0);
        }
        let rows = outlook.top.iter().map(|c| {
            let (artist, title) = name(ctx, &c.file);
            Row::new(vec![
                Cell::from(format!("{:>5.1}%", c.p * 100.0)),
                Cell::from(title),
                Cell::from(artist),
                Cell::from(Span::styled(c.why.clone(), dim)),
            ])
        });
        let header = Row::new(["Chance", "Title", "Artist", "Why"]).style(key);
        let table = Table::new(rows, [
            Constraint::Length(6),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
            Constraint::Min(20),
        ])
        .header(header)
        .column_spacing(1)
        .style(ctx.config.as_text_style())
        .row_highlight_style(ctx.config.theme.current_item_style);
        frame.render_stateful_widget(table, table_area, self.state.as_render_state_ref());

        let foot = if self.files.is_empty() {
            String::new()
        } else {
            format!(
                " the other candidates: {:.1}% together · Enter: Play next · Ctrl-z: heard enough",
                outlook.rest_p * 100.0
            )
        };
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
                    if matches!(event.kind, MouseEventKind::DoubleClick)
                        && let Some(f) = self.selected().cloned()
                    {
                        rormpc_upnext::play_next(ctx, vec![f]);
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
            CommonAction::Top => self.state.first(),
            CommonAction::Bottom => self.state.last(),
            CommonAction::Confirm => {
                if let Some(f) = self.selected().cloned() {
                    rormpc_upnext::play_next(ctx, vec![f]);
                }
            }
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
