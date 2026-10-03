//! rormpc: fzf-like "find in queue". Typing narrows the queue with fuzzy matching (nucleo-matcher), best match
//! first; Enter leaves the input for the list, j/k pick, Enter plays the song (by MPD song id, so the
//! queue is never reordered) and closes; Esc closes without changes.

use anyhow::Result;
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{CaseMatching, Normalization, Pattern},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    symbols::border,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use rmpc_mpd::mpd_client::MpdClient;

use super::Modal;
use crate::{
    config::keys::CommonAction,
    ctx::Ctx,
    shared::{
        id::{self, Id},
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{
        input::{BufferId, InputResultEvent},
        widgets::input::Input,
    },
};

pub struct QueueFindModal {
    id: Id,
    buffer: BufferId,
    matcher: Matcher,
    sel: usize,
    offset: usize,
    list_area: ratatui::layout::Rect,
    /// (song id, label) of the rows shown, in queue order
    shown: Vec<(u32, String)>,
}

impl std::fmt::Debug for QueueFindModal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "QueueFindModal(shown = {})", self.shown.len())
    }
}

impl QueueFindModal {
    pub fn new(ctx: &Ctx) -> Self {
        let buffer = BufferId::new();
        ctx.input.insert_mode(buffer);
        Self {
            id: id::new(),
            buffer,
            matcher: Matcher::new(Config::DEFAULT),
            sel: 0,
            offset: 0,
            list_area: ratatui::layout::Rect::default(),
            shown: Vec::new(),
        }
    }

    /// Queue rows matching the typed text (all rows when empty), in queue order.
    fn refresh(&mut self, ctx: &Ctx) {
        let query = ctx.input.value(self.buffer);
        let pattern = Pattern::parse(query.trim(), CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, usize, (u32, String))> = ctx
            .queue
            .iter()
            .enumerate()
            .filter_map(|(idx, song)| {
                let tag = |k: &str| song.metadata.get(k).map(|v| v.last().to_owned()).unwrap_or_default();
                let label = match (tag("artist"), tag("title")) {
                    (a, t) if !t.is_empty() => format!("{a} - {t}"),
                    _ => song.file.rsplit('/').next().unwrap_or(&song.file).to_owned(),
                };
                if query.trim().is_empty() {
                    return Some((0, idx, (song.id, label)));
                }
                let score = pattern.score(Utf32Str::new(&label, &mut buf), &mut self.matcher)?;
                Some((score, idx, (song.id, label)))
            })
            .collect();
        // best match first like fzf (this picker plays by song id, the queue itself is never reordered);
        // equal scores keep queue order
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        self.shown = scored.into_iter().map(|(_, _, row)| row).collect();
        self.sel = self.sel.min(self.shown.len().saturating_sub(1));
    }

    fn play_selected(&mut self, ctx: &Ctx) -> Result<()> {
        if let Some((id, _)) = self.shown.get(self.sel).cloned() {
            ctx.command(move |_, client| {
                client.play_id(id)?;
                Ok(())
            });
        }
        self.hide(ctx)
    }
}

impl Modal for QueueFindModal {
    fn id(&self) -> Id {
        self.id
    }

    fn render(&mut self, frame: &mut Frame, ctx: &mut Ctx) -> Result<()> {
        self.refresh(ctx);
        let area = frame.area().centered(Constraint::Percentage(70), Constraint::Percentage(70));
        frame.render_widget(Clear, area);
        if let Some(bg) = ctx.config.theme.modal_background_color {
            frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(border::ROUNDED)
            .border_style(ctx.config.as_border_style())
            .title(format!(" Find in queue · {} of {} ", self.shown.len(), ctx.queue.len()));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [input_area, list, help] =
            Layout::vertical([Constraint::Length(3), Constraint::Min(1), Constraint::Length(1)]).areas(inner);
        let input = Input::builder()
            .ctx(ctx)
            .buffer_id(self.buffer)
            .label(" ")
            .label_style(ctx.config.as_text_style())
            .input_style(ctx.config.theme.text_color.map(|c| Style::default().fg(c)).unwrap_or_default())
            .focused(ctx.input.is_insert_mode())
            .focused_style(ctx.config.theme.highlight_border_style)
            .unfocused_style(ctx.config.as_border_style())
            .build();
        frame.render_widget(input, input_area);

        self.list_area = list;
        let height = usize::from(list.height).max(1);
        if self.sel < self.offset {
            self.offset = self.sel;
        } else if self.sel >= self.offset + height {
            self.offset = self.sel + 1 - height;
        }
        let typing = ctx.input.is_insert_mode();
        let lines: Vec<Line> = self
            .shown
            .iter()
            .enumerate()
            .skip(self.offset)
            .take(height)
            .map(|(i, (_, label))| {
                if i == self.sel && !typing {
                    Line::from(Span::styled(format!("› {label}"), ctx.config.theme.current_item_style))
                } else {
                    Line::from(format!("  {label}"))
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), list);
        let hint = if typing {
            "type to filter · Enter: go to the list · Esc: close"
        } else {
            "j/k move · Enter play · i edit filter · Esc close"
        };
        frame.render_widget(Paragraph::new(Span::styled(hint, Style::default().add_modifier(Modifier::DIM))), help);
        Ok(())
    }

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &Ctx) -> Result<()> {
        match kind {
            InputResultEvent::Cancel => self.hide(ctx)?,
            InputResultEvent::Confirm => self.sel = 0, // input left insert mode: now j/k pick a row
            _ => self.sel = 0,
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_key(&mut self, key: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        let Some(action) = key.claim_common().cloned() else { return Ok(()) };
        match action {
            CommonAction::Down => self.sel = (self.sel + 1).min(self.shown.len().saturating_sub(1)),
            CommonAction::Up => self.sel = self.sel.saturating_sub(1),
            CommonAction::Top => self.sel = 0,
            CommonAction::Bottom => self.sel = self.shown.len().saturating_sub(1),
            CommonAction::Confirm => return self.play_selected(ctx),
            CommonAction::FocusInput | CommonAction::EnterSearch => ctx.input.insert_mode(self.buffer),
            CommonAction::Close => return self.hide(ctx),
            _ => return Ok(()),
        }
        ctx.render()?;
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &mut Ctx) -> Result<()> {
        if !self.list_area.contains(event.into()) {
            return Ok(());
        }
        let idx = self.offset + usize::from(event.y.saturating_sub(self.list_area.y));
        if idx < self.shown.len() {
            self.sel = idx;
            if matches!(event.kind, MouseEventKind::DoubleClick) {
                return self.play_selected(ctx);
            }
            ctx.render()?;
        }
        Ok(())
    }
}
