use anyhow::{Context, Result};
use ratatui::{Frame, layout::Position, prelude::Rect, widgets::Widget};

use super::Pane;
use crate::{
    config::tabs::TabName,
    ctx::Ctx,
    shared::{
        events::AppEvent,
        keys::ActionEvent,
        mouse_event::{MouseEvent, MouseEventKind},
    },
    ui::{UiAppEvent, UiEvent, widgets::tabs::Tabs},
};

#[derive(Debug)]
pub struct TabsPane<'a> {
    area: Rect,
    active_tab: TabName,
    tabs: Tabs<'a>,
}

impl TabsPane<'_> {
    pub fn new(ctx: &Ctx) -> Result<Self> {
        let active_tab = Self::init_active_tab(ctx)?;
        let tab_names = Self::init_tab_names(ctx);
        let tabs = Self::init_tabs(tab_names, ctx);

        Ok(Self { area: Rect::default(), active_tab, tabs })
    }

    pub fn get_tab_idx_at(&self, position: Position) -> Option<usize> {
        self.tabs.areas.iter().enumerate().find(|(_, area)| area.contains(position)).map(|v| v.0)
    }

    fn init_active_tab(ctx: &Ctx) -> Result<TabName> {
        // rormpc: start on ctx.active_tab (restored last tab), not always the first tab
        ctx.config.tabs.names.iter().find(|t| **t == ctx.active_tab).cloned().context("Expected at least one tab")
    }

    fn init_tab_names(ctx: &Ctx) -> Vec<String> {
        ctx.config.tabs.names.iter().map(|e| format!("  {e: ^9}  ")).collect::<Vec<String>>()
    }

    fn init_tabs<'a>(tab_names: Vec<String>, ctx: &Ctx) -> Tabs<'a> {
        Tabs::new(tab_names)
            .divider("")
            .style(ctx.config.theme.tab_bar.inactive_style)
            .alignment(ratatui::prelude::Alignment::Center)
            .highlight_style(ctx.config.theme.tab_bar.active_style)
    }
}

impl Pane for TabsPane<'_> {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> anyhow::Result<()> {
        self.area = area;
        if self.area.height > 0 {
            let Some(selected_tab) = ctx
                .config
                .tabs
                .names
                .iter()
                .enumerate()
                .find(|(_, t)| **t == self.active_tab)
                .map(|(idx, _)| idx)
            else {
                return Ok(());
            };

            self.tabs.select(selected_tab);
            self.tabs.render(area, frame.buffer_mut());
        }
        Ok(())
    }

    fn before_show(&mut self, _ctx: &Ctx) -> Result<()> {
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, _is_visible: bool, ctx: &Ctx) -> Result<()> {
        match event {
            UiEvent::TabChanged(tab) => {
                self.active_tab = tab.clone();
                ctx.render()?;
            }
            UiEvent::ConfigChanged => {
                let new_active_tab = ctx
                    .config
                    .tabs
                    .names
                    .iter()
                    .find(|tab| tab == &&self.active_tab)
                    .or(ctx.config.tabs.names.first())
                    .context("Expected at least one tab")
                    .cloned()?;

                let tab_names = Self::init_tab_names(ctx);
                self.tabs = Self::init_tabs(tab_names, ctx);

                self.active_tab = new_active_tab;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if !self.area.contains(event.into()) {
            return Ok(());
        }

        // rormpc: the wheel and the ‹ › markers scroll a bar too narrow for its
        // tabs; the active tab stays
        let is_click =
            matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::DoubleClick);
        let delta = match event.kind {
            MouseEventKind::ScrollUp => -1,
            MouseEventKind::ScrollDown => 1,
            _ if is_click && self.tabs.marker_areas[0].contains(event.into()) => -1,
            _ if is_click && self.tabs.marker_areas[1].contains(event.into()) => 1,
            _ if is_click => 0,
            _ => return Ok(()),
        };
        if delta != 0 {
            self.tabs.scroll(delta);
            return Ok(());
        }

        let Some(tab_name) =
            self.get_tab_idx_at(event.into()).and_then(|idx| ctx.config.tabs.names.get(idx))
        else {
            return Ok(());
        };

        if &self.active_tab == tab_name {
            return Ok(());
        }

        ctx.app_event_sender.send(AppEvent::UiAppEvent(UiAppEvent::ChangeTab(tab_name.clone())))?;

        Ok(())
    }

    fn handle_action(&mut self, _event: &mut ActionEvent, _ctx: &mut Ctx) -> Result<()> {
        Ok(())
    }
}
