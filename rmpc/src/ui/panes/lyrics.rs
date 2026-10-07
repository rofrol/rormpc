use anyhow::Result;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Text,
};

use super::Pane;
use crate::{
    config::{keys::CommonAction, theme::properties::Alignment},
    ctx::{Ctx, LyricsResult},
    shared::{
        ext::duration::DurationExt,
        keys::ActionEvent,
        lrc::{Lrc, LrcOffset},
        macros::status_error,
        mpd_query::run_status_update,
    },
    ui::{
        UiEvent,
        rormpc_filter::binding,
        rormpc_lyrics::{self, Fallback, Translation, View},
    },
};

#[derive(Debug)]
pub struct LyricsPane {
    current_lyrics: Option<Lrc>,
    /// rormpc: plain lyrics or a note when there is no .lrc
    fallback: Option<crate::ui::rormpc_lyrics::Fallback>,
    /// rormpc: the Polish translation beside the lyrics, or a status saying why
    /// there is none
    translation: Option<Translation>,
    /// rormpc: the original line id of each `current_lyrics` line
    line_ids: Vec<usize>,
    /// rormpc: a narrow pane shows the translation instead of the original
    /// (h/l)
    show_polish: bool,
    /// rormpc: `musicdb lyrics translate` runs
    fetching: bool,
    initialized: bool,
    last_requested_line_idx: usize,
}

impl LyricsPane {
    pub fn new(_ctx: &Ctx) -> Self {
        Self {
            current_lyrics: None,
            fallback: None,
            translation: None,
            line_ids: Vec::new(),
            show_polish: false,
            fetching: false,
            initialized: false,
            last_requested_line_idx: 0,
        }
    }

    fn update_lyrics(&mut self, ctx: &Ctx) -> Result<()> {
        self.current_lyrics = None;
        self.translation = None;
        self.line_ids.clear();

        let lrc = ctx.find_lrc()?;
        self.fallback = None;
        let Some((result, lrc)) = lrc else {
            self.fallback = crate::ui::rormpc_lyrics::fallback(ctx);
            if let Some(Fallback::Plain(lines)) = &self.fallback {
                self.translation = rormpc_lyrics::translation(ctx, lines.clone(), true);
            }
            return Ok(());
        };

        if let LyricsResult::Lrc(path) | LyricsResult::Index(path) = &result {
            let (texts, ids) = crate::shared::lrc::timed_line_ids(&std::fs::read_to_string(path)?);
            if ids.len() == lrc.lines.len() {
                self.line_ids = ids;
                self.translation = rormpc_lyrics::translation(ctx, texts, false);
            }
        }
        self.current_lyrics = Some(lrc);
        Ok(())
    }

    /// rormpc: the status row and, with a translation, both columns (or the
    /// chosen one in a narrow pane). Returns the area left for the original
    /// alone, None when everything is drawn.
    fn render_translation(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Option<Rect> {
        let Some(tr) = &self.translation else { return Some(area) };
        let wide = area.width >= rormpc_lyrics::TWO_COLUMNS_MIN_WIDTH;
        let view = match (tr.units.is_empty(), wide, self.show_polish) {
            (true, ..) => View::Original,
            (false, true, _) => View::Both,
            (false, false, false) => View::Original,
            (false, false, true) => View::Polish,
        };
        let nav = &ctx.config.keybinds.navigation;
        let key = |want: fn(&CommonAction) -> bool, fallback: &str| {
            binding(nav, want).unwrap_or_else(|| fallback.to_owned())
        };
        let mut status = if self.fetching {
            "Looking up the Polish translation on tekstowo.pl…".to_owned()
        } else {
            tr.status.clone()
        };
        if let (Some(action), false) = (tr.action, self.fetching) {
            status = format!(
                "{status} · {}: {action}",
                key(|a| matches!(a, CommonAction::Confirm), "Enter")
            );
        }
        if !tr.units.is_empty() && !wide {
            let (left, right) = (
                key(|a| matches!(a, CommonAction::Left), "h"),
                key(|a| matches!(a, CommonAction::Right), "l"),
            );
            status = format!(
                "{status} · {left}/{right}: {}",
                if self.show_polish { "original" } else { "translation" }
            );
        }
        let status_rows =
            textwrap::wrap(&status, usize::from(area.width).max(1)).len().min(3) as u16;
        let body = if area.height > status_rows + 1 && !status.is_empty() {
            let [body, _, status_area] = Layout::vertical([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Length(status_rows),
            ])
            .areas(area);
            rormpc_lyrics::render_status(frame, status_area, &status);
            body
        } else {
            area
        };
        if view == View::Original {
            return Some(body);
        }
        let (current, reached) = if let Some(lrc) = &self.current_lyrics {
            let (idx, reached) = current_line(lrc, ctx.status.elapsed, ctx.config.lyrics_offset);
            schedule_next_line(&mut self.last_requested_line_idx, lrc, idx, ctx);
            (self.line_ids.get(idx).copied(), reached)
        } else {
            let current = rormpc_lyrics::estimated_line(&tr.lines, ctx);
            (current, current.is_some())
        };
        rormpc_lyrics::render_translation(frame, body, ctx, tr, current, reached, view);
        None
    }
}

/// Try to schedule the next line to be displayed on time
fn schedule_next_line(
    last_requested_line_idx: &mut usize,
    lrc: &Lrc,
    current_line_idx: usize,
    ctx: &Ctx,
) {
    let offset = ctx.config.lyrics_offset;
    if *last_requested_line_idx != current_line_idx + 1
        && let Some(line) = lrc.lines.get(current_line_idx + 1)
    {
        *last_requested_line_idx = current_line_idx + 1;
        ctx.scheduler
            .schedule(line.time(offset).saturating_sub(ctx.status.elapsed), run_status_update);
    }
}

/// The line closest to `elapsed` among those already reached, and whether any
/// was reached.
fn current_line(lrc: &Lrc, elapsed: std::time::Duration, offset: LrcOffset) -> (usize, bool) {
    lrc.lines
        .iter()
        .enumerate()
        .filter(|line| elapsed >= line.1.time(offset))
        .min_by(|a, b| a.1.time(offset).abs_diff(elapsed).cmp(&b.1.time(offset).abs_diff(elapsed)))
        .map_or((0, false), |result| (result.0, true))
}

fn align_text(text: Text, alignment: Alignment) -> Text {
    match alignment {
        Alignment::Left => text.left_aligned(),
        Alignment::Right => text.right_aligned(),
        Alignment::Center => text.centered(),
    }
}

impl Pane for LyricsPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        let Some(area) = self.render_translation(frame, area, ctx) else {
            return Ok(());
        };
        let Some(lrc) = &self.current_lyrics else {
            if let Some(fallback) = &self.fallback {
                crate::ui::rormpc_lyrics::render(frame, area, ctx, fallback);
            }
            return Ok(());
        };
        let offset = ctx.config.lyrics_offset;

        let (current_line_idx, first_line_reached) = current_line(lrc, ctx.status.elapsed, offset);

        let rows = area.height;
        let areas = Layout::vertical((0..rows).map(|_| Constraint::Length(1))).split(area);
        let middle_row = rows.saturating_sub(1) / 2;

        let default_style = ctx.config.as_text_style();

        let middle_style = if first_line_reached {
            ctx.config.as_text_style().patch(ctx.config.theme.highlighted_item_style)
        } else {
            default_style
        };

        let timestamp = ctx.config.theme.lyrics.timestamp;

        let Some(current_line) = lrc.lines.get(current_line_idx) else {
            return Ok(());
        };
        let formatted_line = if timestamp && !current_line.content.is_empty() {
            &format!("[{}] {}", current_line.time(offset).to_string(), current_line.content)
        } else {
            &current_line.content
        };

        let wrapped_lines = textwrap::wrap(formatted_line, area.width as usize);
        let wrapped_lines_length = wrapped_lines.len();

        let active_lyric_start_row =
            (middle_row as usize).saturating_sub(wrapped_lines_length.saturating_sub(1));
        let mut current_area = active_lyric_start_row;

        for l in wrapped_lines {
            let Some(area) = areas.get(current_area) else {
                break;
            };
            let text = Text::from(l).style(middle_style);
            frame.render_widget(align_text(text, ctx.config.theme.lyrics.alignment), *area);
            current_area += 1;
        }

        let mut before_lyrics_cursor = current_line_idx;
        let mut before_area_cursor = active_lyric_start_row as usize;
        while before_lyrics_cursor > 0 && before_area_cursor > 0 {
            before_lyrics_cursor -= 1;
            let Some(line) = lrc.lines.get(before_lyrics_cursor) else {
                break;
            };
            let formatted_line = if timestamp && !line.content.is_empty() {
                &format!("[{}] {}", line.time(offset).to_string(), line.content)
            } else {
                &line.content
            };
            for l in textwrap::wrap(formatted_line, area.width as usize).iter().rev() {
                if before_area_cursor == 0 {
                    break;
                }
                let Some(area) = areas.get(before_area_cursor - 1) else {
                    break;
                };
                let text = Text::from(l.as_ref()).style(default_style);

                frame.render_widget(align_text(text, ctx.config.theme.lyrics.alignment), *area);
                before_area_cursor -= 1;
            }
        }
        let mut after_lyrics_cursor = current_line_idx;
        let mut after_area_cursor = current_area.saturating_sub(1);

        while !areas.is_empty()
            && after_lyrics_cursor < lrc.lines.len() - 1
            && after_area_cursor < areas.len() - 1
        {
            after_lyrics_cursor += 1;
            let Some(line) = lrc.lines.get(after_lyrics_cursor) else {
                break;
            };
            let formatted_line = if timestamp && !line.content.is_empty() {
                &format!("[{}] {}", line.time(offset).to_string(), line.content)
            } else {
                &line.content
            };
            for l in textwrap::wrap(formatted_line, area.width as usize) {
                let Some(area) = areas.get(after_area_cursor + 1) else {
                    break;
                };
                let text = Text::from(l).style(default_style);
                frame.render_widget(align_text(text, ctx.config.theme.lyrics.alignment), *area);
                after_area_cursor += 1;
            }
        }

        schedule_next_line(&mut self.last_requested_line_idx, lrc, current_line_idx, ctx);

        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        if !self.initialized {
            if let Err(err) = self.update_lyrics(ctx) {
                status_error!("Failed to load lyrics file: '{err}'");
            }
            self.last_requested_line_idx = 0;
            self.initialized = true;
        }

        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, _is_visible: bool, ctx: &Ctx) -> Result<()> {
        match event {
            UiEvent::SongChanged | UiEvent::Reconnected | UiEvent::LyricsIndexed => {
                self.fetching = false;
                if let Err(err) = self.update_lyrics(ctx) {
                    status_error!("Failed to load lyrics file: '{err}'");
                }
                ctx.render()?;
                self.last_requested_line_idx = 0;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        // rormpc: Enter looks the Polish translation up, h/l switches original
        // / translation in a narrow pane
        let Some(tr) = &self.translation else { return Ok(()) };
        match event.claim_common() {
            Some(CommonAction::Confirm) if tr.action.is_some() && !self.fetching => {
                self.fetching = true;
                rormpc_lyrics::fetch_translation(ctx);
                ctx.render()?;
            }
            Some(CommonAction::Left | CommonAction::Right) if !tr.units.is_empty() => {
                self.show_polish = !self.show_polish;
                ctx.render()?;
            }
            Some(_) => event.abandon(),
            None => {}
        }
        Ok(())
    }
}
