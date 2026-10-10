use itertools::Itertools;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    widgets::{Row, StatefulWidget, Table, TableState},
};

use crate::ui::dirstack::DirState;

/// A simple wrapper around ratatui's Table widget which virtualizes the rows
/// iterator to only materialize the rows necessary for rendering. This is why
/// this table only takes Iterator and not `IntoIterator`.
#[derive(Debug)]
pub struct VirtualizedTable<'a, 'song, T, F>
where
    F: Fn(usize, &'song T) -> Row<'a>,
{
    items: &'song [T],
    column_widths: Vec<Constraint>,
    map_fn: Option<F>,
    /// rormpc: an extra row painted before item `.0` (Music's "Up next · N"); `.2`: the cursor is on it. The
    /// caller keeps the state's viewport one row shorter while it is set, so the gap never pushes a row out.
    gap: Option<(usize, Row<'a>, bool)>,
}

impl<'a, 'song, T, F> VirtualizedTable<'a, 'song, T, F>
where
    F: Fn(usize, &'song T) -> Row<'a>,
{
    pub fn new(items: &'song [T]) -> Self {
        Self { items, column_widths: Vec::new(), map_fn: None, gap: None }
    }

    pub fn map_fn(mut self, f: F) -> Self {
        self.map_fn = Some(f);
        self
    }

    /// rormpc: paint `row` before item `at`, the cursor on it when `selected`.
    pub fn gap(mut self, at: usize, row: Row<'a>, selected: bool) -> Self {
        self.gap = Some((at, row, selected));
        self
    }

    pub fn column_widths<W>(mut self, widths: W) -> Self
    where
        W: IntoIterator,
        W::Item: Into<Constraint>,
    {
        self.column_widths = widths.into_iter().map(Into::into).collect_vec();
        self
    }
}

impl<'a, 'song, T, F> StatefulWidget for VirtualizedTable<'a, 'song, T, F>
where
    F: Fn(usize, &'song T) -> Row<'a>,
{
    type State = DirState<TableState>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State)
    where
        Self: Sized,
    {
        let Some(viewport_len) = state.viewport_len() else {
            return;
        };
        let Some(map_fn) = self.map_fn.as_ref() else {
            return;
        };

        // Save original state and remove offset because ratatui's table will
        // think that we are rendering from item 0 to viewport_len, the
        // rest will be ignored
        let original_offset = state.offset();
        let original_selected = state.inner.selected();
        *state.inner.offset_mut() = 0;

        let mut actual_rows = self
            .items
            .iter()
            .skip(original_offset)
            .take(viewport_len)
            .enumerate()
            .map(|(idx, item)| map_fn(idx + original_offset, item))
            .collect_vec();
        let mut selected = original_selected.map(|v| v.saturating_sub(original_offset));
        if let Some((at, row, on_gap)) = self.gap
            && at >= original_offset
            && at <= original_offset + actual_rows.len()
        {
            let gap_row = at - original_offset;
            actual_rows.insert(gap_row, row);
            selected = if on_gap {
                Some(gap_row)
            } else {
                original_selected.map(|v| {
                    let v = v.saturating_sub(original_offset);
                    if v >= gap_row { v + 1 } else { v }
                })
            };
        }
        // straight on the table state: with the gap the cursor row may be the viewport's last + 1
        state.inner.select(selected);
        let table = Table::new(actual_rows, self.column_widths);

        StatefulWidget::render(table, area, buf, state.as_render_state_ref());

        // Restore the original state
        *state.inner.offset_mut() = original_offset;
        state.select(original_selected, 0);
    }
}
