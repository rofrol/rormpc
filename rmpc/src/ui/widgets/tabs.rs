// This is a "fork" of ratatui's Tabs widget

// The MIT License (MIT)
//
// Copyright (c) 2016-2022 Florian Dehau
// Copyright (c) 2023 The Ratatui Developers
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.
use std::ops::Range;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    prelude::Alignment,
    style::{Style, Styled},
    symbols,
    text::{Line, Span},
    widgets::{Block, Widget},
};

use super::get_line_offset;

/// A widget to display available tabs in a multiple panels context.
///
/// # Examples
///
/// ```
/// # use ratatui::widgets::{Block, Borders, Tabs};
/// # use ratatui::style::{Style, Color};
/// # use ratatui::text::{Line};
/// # use ratatui::symbols::{DOT};
/// let titles = ["Tab1", "Tab2", "Tab3", "Tab4"].iter().cloned().map(Line::from).collect();
/// Tabs::new(titles)
///     .block(Block::default().title("Tabs").borders(Borders::ALL))
///     .style(Style::default().fg(Color::White))
///     .highlight_style(Style::default().fg(Color::Yellow))
///     .divider(DOT);
/// ```
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Tabs<'a> {
    /// A block to wrap this widget in if necessary
    pub block: Option<Block<'a>>,
    /// One title for each tab
    pub titles: Vec<Line<'a>>,
    /// The index of the selected tabs
    selected: usize,
    /// The style used to draw the text
    pub style: Style,
    /// Style to apply to the selected item
    pub highlight_style: Style,
    /// Tab divider
    pub divider: Span<'a>,
    /// Alignment of the tabs
    pub alignment: Alignment,
    /// Vec of areas that tabs were last rendered in
    pub areas: Vec<Rect>,
    /// rormpc: the first tab shown when the titles do not fit the bar
    offset: usize,
    /// rormpc: the selected tab is scrolled into view on the next render (it or
    /// the bar's width changed)
    follow_selected: bool,
    last_width: u16,
    /// rormpc: the ‹ and › markers' areas in the last paint (empty when that
    /// side hides no tab)
    pub marker_areas: [Rect; 2],
}

#[allow(unused)]
impl<'a> Tabs<'a> {
    pub fn new<T>(titles: Vec<T>) -> Tabs<'a>
    where
        T: Into<Line<'a>>,
    {
        let titles: Vec<_> = titles.into_iter().map(Into::into).collect();
        Tabs {
            block: None,
            selected: 0,
            style: Style::default(),
            highlight_style: Style::default(),
            divider: Span::raw(symbols::line::VERTICAL),
            alignment: Alignment::Left,
            areas: vec![Rect::default(); titles.len()],
            titles,
            offset: 0,
            follow_selected: true,
            last_width: 0,
            marker_areas: [Rect::default(); 2],
        }
    }

    pub fn block(mut self, block: Block<'a>) -> Tabs<'a> {
        self.block = Some(block);
        self
    }

    pub fn select(&mut self, selected: usize) -> &mut Self {
        self.follow_selected |= self.selected != selected;
        self.selected = selected;
        self
    }

    /// rormpc: moves the overflowing bar by `delta` tabs; the next render
    /// clamps it to the tabs that exist.
    pub fn scroll(&mut self, delta: isize) {
        self.offset = self.offset.saturating_add_signed(delta);
    }

    pub fn style(mut self, style: Style) -> Tabs<'a> {
        self.style = style;
        self
    }

    pub fn titles(&mut self, titles: Vec<impl Into<Line<'a>>>) -> &mut Tabs<'a> {
        let titles: Vec<_> = titles.into_iter().map(Into::into).collect();
        self.titles = titles;
        self
    }

    pub fn highlight_style(mut self, style: Style) -> Tabs<'a> {
        self.highlight_style = style;
        self
    }

    pub fn divider<T>(mut self, divider: T) -> Tabs<'a>
    where
        T: Into<Span<'a>>,
    {
        self.divider = divider.into();
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Tabs<'a> {
        self.alignment = alignment;
        self
    }
}

impl<'a> Styled for Tabs<'a> {
    type Item = Tabs<'a>;

    fn style(&self) -> Style {
        self.style
    }

    fn set_style<S: Into<Style>>(self, style: S) -> Self::Item {
        self.style(style.into())
    }
}

impl Widget for &mut Tabs<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        buf.set_style(area, self.style);
        let tabs_area = match &self.block {
            Some(b) => {
                let inner_area = b.inner(area);
                b.render(area, buf);
                inner_area
            }
            None => area,
        };

        if tabs_area.height < 1 || tabs_area.width < 1 {
            return;
        }

        let widths: Vec<u16> = self
            .titles
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let divider =
                    if i + 1 < self.titles.len() { self.divider.width() as u16 } else { 0 };
                t.width() as u16 + divider
            })
            .collect();
        self.marker_areas = [Rect::default(); 2];
        // rormpc: titles that do not fit scroll instead of being cut at the
        // right edge
        let (range, mut x, right) = if widths.iter().sum::<u16>() <= tabs_area.width {
            self.offset = 0;
            let x = get_line_offset(
                self.titles.iter().map(|t| t.width() as u16).sum(),
                tabs_area.width,
                self.alignment,
            ) + area.x;
            (0..self.titles.len(), x, tabs_area.right())
        } else {
            let follow = (self.follow_selected || self.last_width != tabs_area.width)
                .then_some(self.selected);
            let range =
                visible_range(&widths, tabs_area.width.saturating_sub(2), self.offset, follow);
            self.offset = range.start;
            let y = tabs_area.top();
            if range.start > 0 {
                self.marker_areas[0] = Rect { x: tabs_area.x, y, width: 1, height: 1 };
                buf.set_string(tabs_area.x, y, "‹", self.style);
            }
            if range.end < self.titles.len() {
                self.marker_areas[1] = Rect { x: tabs_area.right() - 1, y, width: 1, height: 1 };
                buf.set_string(tabs_area.right() - 1, y, "›", self.style);
            }
            (range, tabs_area.x + 1, tabs_area.right() - 1)
        };
        self.follow_selected = false;
        self.last_width = tabs_area.width;

        self.areas.iter_mut().for_each(|a| *a = Rect::default());
        let last_visible = range.end.saturating_sub(1);
        for i in range {
            let remaining_width = right.saturating_sub(x);
            if remaining_width == 0 {
                break;
            }
            let pos = buf.set_line(x, tabs_area.top(), &self.titles[i], remaining_width);
            self.areas[i] =
                Rect { x, y: tabs_area.top(), width: pos.0.saturating_sub(x), height: 1 };

            if i == self.selected {
                buf.set_style(self.areas[i], self.highlight_style);
            }
            x = pos.0.saturating_add(1);
            if right.saturating_sub(x) == 0 || i == last_visible {
                break;
            }
            let pos = buf.set_span(
                x.saturating_sub(1),
                tabs_area.top(),
                &self.divider,
                self.divider.width() as u16,
            );
            x = pos.0;
        }
    }
}

/// rormpc: the tabs (with their `widths`) that fit in `width` columns, starting
/// at `offset`, or moved just enough to show `follow`. The start never leaves
/// empty space at the end, and at least one tab is shown.
fn visible_range(widths: &[u16], width: u16, offset: usize, follow: Option<usize>) -> Range<usize> {
    let end_from = |start: usize| {
        let mut used = 0u16;
        let mut end = start;
        while end < widths.len() && (end == start || used + widths[end] <= width) {
            used = used.saturating_add(widths[end]);
            end += 1;
        }
        end
    };
    // the first start whose tabs reach the last one: `end` fits `width` from
    // `start` onwards
    let start_for_end = |end: usize| {
        let mut used = 0u16;
        let mut start = end;
        while start > 0 && used + widths[start - 1] <= width {
            used += widths[start - 1];
            start -= 1;
        }
        start.min(end.saturating_sub(1))
    };
    let mut start = offset.min(start_for_end(widths.len()));
    if let Some(selected) = follow.filter(|s| *s < widths.len()) {
        if selected < start {
            start = selected;
        } else if selected >= end_from(start) {
            start = start_for_end(selected + 1);
        }
    }
    start..end_from(start)
}

#[cfg(test)]
mod tests {
    use ratatui::style::{Color, Modifier, Stylize};

    use super::*;

    #[test]
    fn can_be_stylized() {
        assert_eq!(
            Tabs::new(vec![""]).black().on_white().bold().not_italic().style,
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
                .remove_modifier(Modifier::ITALIC)
        );
    }

    #[test]
    fn visible_range_scrolls_to_the_selected_tab_and_clamps_the_offset() {
        let widths = [4, 4, 4, 4, 4];
        // everything fits
        assert_eq!(visible_range(&widths, 20, 3, None), 0..5);
        // the offset never leaves empty space at the end
        assert_eq!(visible_range(&widths, 10, 0, None), 0..2);
        assert_eq!(visible_range(&widths, 10, 2, None), 2..4);
        assert_eq!(visible_range(&widths, 10, 9, None), 3..5);
        // a selected tab to the right becomes the last shown, one to the left
        // the first
        assert_eq!(visible_range(&widths, 10, 0, Some(3)), 2..4);
        assert_eq!(visible_range(&widths, 10, 3, Some(1)), 1..3);
        // a visible selected tab does not move the bar
        assert_eq!(visible_range(&widths, 10, 1, Some(2)), 1..3);
        // a tab wider than the bar is still shown (clipped)
        assert_eq!(visible_range(&[30, 4], 10, 0, Some(0)), 0..1);
        assert_eq!(visible_range(&[30, 4], 10, 0, Some(1)), 1..2);
    }

    #[test]
    fn overflowing_bar_shows_markers_and_keeps_click_areas_of_drawn_tabs() {
        let mut tabs = Tabs::new(vec!["aaaa", "bbbb", "cccc", "dddd"]).divider("");
        tabs.select(2);
        let area = Rect::new(0, 0, 10, 1);
        let mut buf = Buffer::empty(area);
        (&mut tabs).render(area, &mut buf);
        assert_eq!(buf, Buffer::with_lines(["‹bbbbcccc›"]));
        assert_eq!(tabs.areas[0], Rect::default());
        assert_eq!(tabs.areas[2], Rect::new(5, 0, 4, 1));
        assert_eq!(tabs.marker_areas, [Rect::new(0, 0, 1, 1), Rect::new(9, 0, 1, 1)]);

        // the wheel moves the bar without changing the selection; the next
        // render clamps it
        tabs.scroll(5);
        let mut buf = Buffer::empty(area);
        (&mut tabs).render(area, &mut buf);
        assert_eq!(buf, Buffer::with_lines(["‹ccccdddd "]));
        assert_eq!(tabs.areas[3], Rect::new(5, 0, 4, 1));
    }
}
