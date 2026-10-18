use std::time::Duration;

use anyhow::Result;
use ratatui::{Frame, prelude::Rect, widgets::Paragraph};
use rmpc_mpd::{
    commands::State,
    mpd_client::{MpdClient, ValueChange},
};

use super::Pane;
use crate::{
    ctx::Ctx,
    shared::{
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::widgets::progress_bar::ProgressBar,
};

#[derive(Debug)]
pub struct ProgressBarPane {
    area: Rect,
}

impl ProgressBarPane {
    pub fn new() -> Self {
        Self { area: Rect::default() }
    }
}

impl Pane for ProgressBarPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> anyhow::Result<()> {
        self.area = area;

        match ctx.messages.last() {
            Some(status) if status.created.elapsed() < status.timeout => {
                let status_bar = Paragraph::new(status.message.clone())
                    .alignment(ratatui::prelude::Alignment::Center)
                    .style(status.level.into_style(&ctx.config.theme.level_styles));
                frame.render_widget(status_bar, self.area);
            }
            _ if ctx.hover_hint.borrow().is_some() => {
                let hint = Paragraph::new(ctx.hover_hint.borrow().clone().unwrap_or_default())
                    .alignment(ratatui::prelude::Alignment::Center);
                frame.render_widget(hint, self.area);
            }
            _ => {
                let bar_cfg = &ctx.config.theme.progress_bar;
                let value = if ctx.status.duration == Duration::ZERO {
                    0.0
                } else {
                    ctx.status.elapsed.as_secs_f32() / ctx.status.duration.as_secs_f32()
                };
                let bar = ProgressBar::builder()
                    .elapsed_style(bar_cfg.elapsed_style)
                    .thumb_style(bar_cfg.thumb_style)
                    .track_style(bar_cfg.track_style)
                    .start_char(&bar_cfg.symbols[0])
                    .elapsed_char(&bar_cfg.symbols[1])
                    .thumb_char(&bar_cfg.symbols[2])
                    .track_char(&bar_cfg.symbols[3])
                    .end_char(&bar_cfg.symbols[4])
                    .use_track_when_empty(ctx.config.theme.progress_bar.use_track_when_empty)
                    .value(value)
                    .build();

                frame.render_widget(bar, self.area);
            }
        }

        Ok(())
    }

    fn before_show(&mut self, _ctx: &Ctx) -> Result<()> {
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if !self.area.contains(event.into()) {
            return Ok(());
        }

        match event.kind {
            MouseEventKind::LeftClick | MouseEventKind::DoubleClick
                if matches!(ctx.status.state, State::Play | State::Pause) =>
            {
                let second_to_seek_to = ctx
                    .status
                    .duration
                    .mul_f32(
                        f32::from(event.x.saturating_sub(self.area.x)) / f32::from(self.area.width),
                    )
                    .as_secs();
                ctx.command(move |_, client| {
                    client.seek_current(ValueChange::Set(u32::try_from(second_to_seek_to)?))?;
                    Ok(())
                });

                ctx.render()?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_action(&mut self, _event: &mut ActionEvent, _ctx: &mut Ctx) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod hover_hint_tests {
    use super::*;
    use crate::{shared::events::Level, tests::fixtures::ctx, ui::StatusMessage};
    use ratatui::{Terminal, backend::TestBackend};
    use rstest::rstest;
    use std::time::Instant;

    fn paint(pane: &mut ProgressBarPane, ctx: &Ctx) -> String {
        let mut terminal = Terminal::new(TestBackend::new(40, 1)).unwrap();
        terminal.draw(|frame| pane.render(frame, frame.area(), ctx).unwrap()).unwrap();
        terminal.backend().buffer().content().iter().map(|cell| cell.symbol()).collect()
    }

    #[rstest]
    fn leaving_a_composite_child_clears_without_asking_that_child_to_handle_the_move(ctx: Ctx) {
        let area = Rect::new(10, 2, 2, 1);
        ctx.set_like_hint("hits", Some("song.flac"), area).unwrap();
        ctx.clear_hover_outside((11, 2).into()).unwrap();
        assert_eq!(ctx.hover_hint.borrow().as_deref(), Some("click: like"));
        ctx.clear_like_hint("queue").unwrap();
        assert!(ctx.hover_hint.borrow().is_some(), "a different pane cannot clear the owner");
        ctx.clear_hover_outside((12, 2).into()).unwrap();
        assert_eq!(*ctx.hover_hint.borrow(), None);
        assert!(!ctx.refresh_like_hint("hits", Some("song.flac")), "a repaint cannot resurrect a departed hover");
        assert_eq!(*ctx.hover_hint.borrow(), None);
    }

    #[rstest]
    fn only_hint_changes_request_a_redraw(mut ctx: Ctx) {
        let (tx, rx) = crossbeam::channel::unbounded();
        ctx.app_event_sender = tx;
        ctx.set_hover_hint(Some("click: like".to_owned())).unwrap();
        assert!(matches!(rx.try_recv().unwrap(), crate::shared::events::AppEvent::HoverHintChanged));
        ctx.set_hover_hint(Some("click: like".to_owned())).unwrap();
        assert!(rx.try_recv().is_err());
        ctx.set_hover_hint(None).unwrap();
        assert!(matches!(rx.try_recv().unwrap(), crate::shared::events::AppEvent::HoverHintChanged));
        ctx.set_hover_hint(None).unwrap();
        paint(&mut ProgressBarPane::new(), &ctx);
        assert!(rx.try_recv().is_err(), "clearing and repainting leave no extra redraw pending");
    }

    #[rstest]
    fn hint_lives_until_cleared_and_never_hides_a_live_status(mut ctx: Ctx) {
        let mut pane = ProgressBarPane::new();
        let empty = paint(&mut pane, &ctx);
        ctx.set_hover_hint(Some("click: like".to_owned())).unwrap();
        assert!(paint(&mut pane, &ctx).contains("click: like"));
        ctx.messages.push(StatusMessage {
            message: "important error".to_owned(), level: Level::Error,
            created: Instant::now(), timeout: Duration::MAX,
        });
        let live = paint(&mut pane, &ctx);
        assert!(live.contains("important error"));
        assert!(!live.contains("click:"));
        ctx.messages.clear();
        ctx.messages.push(StatusMessage {
            message: "expired".to_owned(), level: Level::Info,
            created: Instant::now(), timeout: Duration::ZERO,
        });
        assert!(paint(&mut pane, &ctx).contains("click: like"));
        ctx.set_hover_hint(None).unwrap();
        assert_eq!(paint(&mut pane, &ctx), empty);
    }
}
