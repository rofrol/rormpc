//! rormpc: status messages wider than the progress bar pane. Upstream draws the message in the pane alone and cuts it
//! at its width ("Playing from … · N songs in their order, prepared in 0.4 s" lost its end). Such a message is drawn
//! again after all panes, across the whole terminal width, wrapped onto as many rows as it needs: the bar grows upward
//! over the rows above it while the message shows (downward when the bar is on the top row). Its timeout grows with
//! its length, so there is time to read it.

use std::time::Duration;

use ratatui::{
    Frame,
    layout::{Alignment, Margin, Rect},
    style::Style,
    widgets::{Block, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::ctx::Ctx;

/// a short message stays as long as upstream showed every message
const MIN_READ: Duration = Duration::from_secs(5); // delay: not a wait, reading time for a person
/// so a long message does not hide the progress bar for good
const MAX_READ: Duration = Duration::from_secs(20); // delay: not a wait, reading time for a person
/// time to notice the message and move the eyes to the bar before reading starts
const NOTICE: Duration = Duration::from_secs(2); // delay: not a wait, reading time for a person
/// reading speed in terminal columns (characters) per second: about 180 words a minute, below the usual silent
/// reading speed, as the message is read while doing something else
const CHARS_PER_SECOND: f64 = 15.0;
/// most rows a wrapped message takes; the rest is cut with "…"
const MAX_ROWS: usize = 4;

/// How long a status message without its own timeout stays on screen: `clamp(NOTICE + width / CHARS_PER_SECOND,
/// MIN_READ, MAX_READ)`, i.e. 5 s up to 45 columns, 7.3 s for 80 and 20 s from 270. This is reading time for a
/// person, not a wait for an asynchronous event.
pub fn reading_time(message: &str) -> Duration {
    let reading = Duration::from_secs_f64(message.width() as f64 / CHARS_PER_SECOND);
    (NOTICE + reading).clamp(MIN_READ, MAX_READ)
}

/// `message` wrapped to `width` columns, at most `max_rows` rows; a cut last row ends with "…".
pub fn wrap(message: &str, width: u16, max_rows: usize) -> Vec<String> {
    let width = usize::from(width.max(1));
    let max_rows = max_rows.max(1);
    let mut rows: Vec<String> = textwrap::wrap(message, width).into_iter().map(|row| row.into_owned()).collect();
    if rows.len() > max_rows {
        rows.truncate(max_rows);
        let last = rows.last_mut().expect("at least one row");
        while last.width() + 1 > width && last.pop().is_some() {}
        last.push('…');
    }
    rows
}

/// The rows a message of `rows` rows takes when the bar is at `bar` in a frame of `frame`: the bar's row and the
/// rows above it, or below it when there are not enough rows above.
pub fn overflow_area(frame: Rect, bar: Rect, rows: u16) -> Rect {
    let rows = rows.clamp(1, frame.height.max(1));
    let above = bar.y.saturating_sub(frame.y);
    let y = if above + 1 >= rows { bar.y + 1 - rows } else { bar.y.min(frame.bottom().saturating_sub(rows)) };
    Rect::new(frame.x, y, frame.width, rows)
}

/// Draws the current status message across the whole width when it is wider than the bar pane at `bar`. Call after
/// all panes are rendered and before modals.
pub fn render_overflow(frame: &mut Frame, bar: Rect, ctx: &Ctx) {
    let Some(status) = ctx.messages.last() else { return };
    if status.created.elapsed() >= status.timeout || status.message.width() <= usize::from(bar.width) {
        return;
    }
    let full = frame.area();
    // one empty column on each side, like the Empty panes around the user's bar
    let width = full.width.saturating_sub(2).max(1);
    let rows = wrap(&status.message, width, MAX_ROWS.min(usize::from(full.height)));
    let area = overflow_area(full, bar, u16::try_from(rows.len()).unwrap_or(u16::MAX));
    frame.render_widget(Clear, area);
    if let Some(bg) = ctx.config.theme.background_color {
        frame.render_widget(Block::default().style(Style::default().bg(bg)), area);
    }
    let text = Paragraph::new(rows.join("\n"))
        .alignment(Alignment::Center)
        .style(status.level.into_style(&ctx.config.theme.level_styles));
    frame.render_widget(text, area.inner(Margin::new(1, 0)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s) // delay: not a wait, expected reading time in a test
    }

    #[test]
    fn short_messages_keep_five_seconds() {
        assert_eq!(reading_time("Added 3 songs"), MIN_READ);
        assert_eq!(reading_time(&"x".repeat(45)), MIN_READ);
    }

    #[test]
    fn long_messages_get_reading_time() {
        // 2 s + 90 / 15 s
        assert_eq!(reading_time(&"x".repeat(90)), secs(8));
        assert_eq!(reading_time(&"x".repeat(1000)), MAX_READ);
    }

    #[test]
    fn reading_time_counts_columns_not_bytes() {
        // 90 columns, 180 bytes
        assert_eq!(reading_time(&"ł".repeat(90)), secs(8));
    }

    #[test]
    fn wrap_keeps_a_fitting_message_on_one_row() {
        assert_eq!(wrap("Playing", 20, 4), vec!["Playing"]);
    }

    #[test]
    fn wrap_breaks_at_words() {
        let msg = "Playing from Hits · 11 songs in their order, prepared in 0.4 s";
        let rows = wrap(msg, 40, 4);
        assert_eq!(rows, vec!["Playing from Hits · 11 songs in their", "order, prepared in 0.4 s"]);
        assert!(rows.iter().all(|r| r.width() <= 40));
    }

    #[test]
    fn wrap_cuts_beyond_max_rows_with_ellipsis() {
        assert_eq!(wrap("aaaa bbbb cccc dddd", 4, 2), vec!["aaaa", "bbb…"]);
    }

    #[test]
    fn overflow_grows_upward_from_the_bar() {
        let frame = Rect::new(0, 0, 80, 30);
        let bar = Rect::new(1, 25, 30, 1);
        assert_eq!(overflow_area(frame, bar, 2), Rect::new(0, 24, 80, 2));
        assert_eq!(overflow_area(frame, bar, 1), Rect::new(0, 25, 80, 1));
    }

    #[test]
    fn overflow_grows_downward_from_a_top_bar() {
        let frame = Rect::new(0, 0, 80, 30);
        let bar = Rect::new(1, 0, 30, 1);
        assert_eq!(overflow_area(frame, bar, 3), Rect::new(0, 0, 80, 3));
    }

    #[test]
    fn overflow_stays_inside_a_short_frame() {
        let frame = Rect::new(0, 0, 80, 2);
        let bar = Rect::new(1, 1, 30, 1);
        assert_eq!(overflow_area(frame, bar, 4), Rect::new(0, 0, 80, 2));
    }
}
