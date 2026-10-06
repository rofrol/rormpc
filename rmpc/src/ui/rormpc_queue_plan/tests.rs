use std::sync::Arc;

use ratatui::{Terminal, backend::TestBackend};
use rstest::rstest;
use serde_json::json;

use super::*;
use crate::{
    tests::fixtures::ctx,
    ui::panes::{Pane, queue::QueuePane},
};

fn songs() -> Vec<Song> {
    (1..=18).map(|id| Song { id, file: format!("s{id}"), ..Default::default() }).collect()
}
fn shuffle(ids: &[u32]) -> ShuffleState {
    serde_json::from_value(json!({"enabled": true, "active": true, "plan_version": "session:1", "updated_at": now() - 1.0,
        "plan": ids.iter().map(|id| json!({"id":id, "file":format!("s{id}")})).collect::<Vec<_>>() })).unwrap()
}
fn publish(sh: ShuffleState) {
    rormpc_player::TEST_SHUFFLE.with(|s| *s.borrow_mut() = sh);
}
fn common(action: CommonAction) -> ActionEvent {
    ActionEvent::from(Arc::new(vec![action.into()]))
}
fn queue(action: QueueActions) -> ActionEvent {
    ActionEvent::from(Arc::new(vec![action.into()]))
}
fn live(ctx: &mut Ctx) {
    ctx.queue = songs();
    ctx.player_present.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[test]
fn sections_include_all_ten_slots_and_physical_order_tail() {
    let mut sh = shuffle(&(8..=17).collect::<Vec<_>>());
    sh.history =
        serde_json::from_value(json!([{"id":2,"file":"s2"},{"id":3,"file":"s3"}])).unwrap();
    let requests = vec![Waiting { id: 5, file: "s5".into(), added: false }, Waiting {
        id: 6,
        file: "s6".into(),
        added: false,
    }];
    let rows = project(&songs(), Some(4), &requests, &sh);
    assert_eq!(rows.len(), 19);
    assert_eq!(rows[..5].iter().map(|r| r.turn).collect::<Vec<_>>(), vec![
        Turn::Past(-2),
        Turn::Past(-1),
        Turn::Current,
        Turn::Request(1),
        Turn::Request(2)
    ]);
    assert_eq!(
        rows[5..15].iter().map(|r| r.turn).collect::<Vec<_>>(),
        (1..=10).map(Turn::Forecast).collect::<Vec<_>>()
    );
    assert_eq!(rows[15].turn, Turn::Divider);
    assert_eq!(rows[16..].iter().map(|r| r.id.unwrap()).collect::<Vec<_>>(), vec![1, 7, 18]);
}

#[test]
fn id_reuse_and_duplicate_roles_never_duplicate_rows() {
    let mut sh = shuffle(&[1, 2, 3]);
    sh.plan[0].file = "old-file".into();
    sh.history =
        serde_json::from_value(json!([{"id":2,"file":"s2"},{"id":2,"file":"s2"}])).unwrap();
    let requests = vec![Waiting { id: 2, file: "s2".into(), added: false }];
    let rows = project(&songs(), Some(3), &requests, &sh);
    assert_eq!(rows.iter().filter(|r| r.id == Some(2)).count(), 1);
    assert_eq!(rows.iter().find(|r| r.id == Some(2)).unwrap().turn, Turn::Request(1));
    assert_eq!(rows.iter().find(|r| r.id == Some(3)).unwrap().turn, Turn::Current);
    assert_eq!(rows.iter().find(|r| r.id == Some(1)).unwrap().turn, Turn::Unplanned);
}

#[test]
fn absent_old_missing_future_and_failed_publications_are_stale() {
    let mut sh = shuffle(&[1, 2]);
    let t = now();
    assert!(!stale(&sh, true, t));
    assert!(stale(&sh, false, t));
    sh.updated_at = t - 60.0;
    assert!(stale(&sh, true, t));
    sh.updated_at = t + 0.1;
    assert!(stale(&sh, true, t));
    sh.updated_at = 0.0;
    assert!(stale(&sh, true, t));
    sh.updated_at = t - 1.0;
    sh.plan_version.clear();
    assert!(stale(&sh, true, t));
    sh.plan_version = "session:2".into();
    sh.publish_error = Some("failed".into());
    assert!(stale(&sh, true, t));
}

#[rstest]
fn redraw_preserves_selected_id_and_top_visible_id(mut ctx: Ctx) {
    live(&mut ctx);
    let mut sh = shuffle(&(1..=10).collect::<Vec<_>>());
    publish(sh.clone());
    let mut view = PlanView::new();
    view.area = Rect::new(0, 0, 80, 8);
    view.select_id(Some(7));
    view.refresh(&ctx, false);
    view.state.set_offset(4);
    let top = view.rows[4].id;
    sh.plan.rotate_left(2);
    sh.plan_version = "session:2".into();
    publish(sh);
    view.refresh(&ctx, false);
    assert_eq!(view.selected_id, Some(7));
    assert_eq!(view.rows[view.state.offset()].id, top);
}

#[rstest]
fn filtering_keeps_original_turn_number_and_escape_restores_id(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&(8..=17).collect::<Vec<_>>()));
    let mut view = PlanView::new();
    view.select_id(Some(10));
    view.refresh(&ctx, false);
    view.search(&ctx);
    view.query = "s17".into();
    view.refresh(&ctx, true);
    assert_eq!(view.rows, vec![PlanRow { id: Some(17), turn: Turn::Forecast(10) }]);
    view.insert(&InputResultEvent::Cancel, &ctx).unwrap();
    assert_eq!(view.selected_id, Some(10));
}

#[rstest]
fn divider_never_selected_by_top_arrows_marks_or_click(mut ctx: Ctx) {
    live(&mut ctx);
    publish(ShuffleState::default());
    let mut view = PlanView::new();
    view.area = Rect::new(0, 0, 80, 20);
    view.refresh(&ctx, false);
    view.action(&mut common(CommonAction::Top), &mut ctx).unwrap();
    assert_eq!(view.selected_id, Some(1));
    view.action(&mut common(CommonAction::Up), &mut ctx).unwrap();
    assert!(view.selected_id.is_some());
    view.action(&mut common(CommonAction::InvertSelection), &mut ctx).unwrap();
    assert_eq!(view.marked.len(), 18);
    let selected = view.selected_id;
    for kind in [
        MouseEventKind::LeftClick,
        MouseEventKind::DoubleClick,
        MouseEventKind::RightClick,
        MouseEventKind::MiddleClick,
    ] {
        view.mouse(MouseEvent { x: 0, y: 0, kind }, &ctx).unwrap();
        assert_eq!(view.selected_id, selected);
    }
}

#[rstest]
fn marking_moves_down_but_skips_the_divider(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1]));
    let mut view = PlanView::new();
    view.area = Rect::new(0, 0, 80, 20);
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    view.action(&mut common(CommonAction::Select), &mut ctx).unwrap();
    assert!(view.marked.contains(&1));
    assert_eq!(view.selected_id, Some(2));
    view.action(&mut common(CommonAction::Close), &mut ctx).unwrap();
    assert!(view.marked.is_empty());
}

#[rstest]
fn removed_cursor_and_marks_resolve_to_surviving_song(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 2, 3]));
    let mut view = PlanView::new();
    view.select_id(Some(2));
    view.marked.insert(2);
    view.area = Rect::new(0, 0, 80, 20);
    view.refresh(&ctx, false);
    ctx.queue.retain(|s| s.id != 2);
    publish(shuffle(&[1, 3]));
    view.refresh(&ctx, false);
    assert_eq!(view.selected_id, Some(3));
    assert!(!view.marked.contains(&2));
}

#[rstest]
fn duplicate_forecast_is_stale_and_cannot_be_patched(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 1, 2]));
    let mut view = PlanView::new();
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    assert!(view.is_stale);
    view.move_selected(1, &ctx);
    assert!(view.pending.is_none());
}

#[rstest]
fn pending_swap_is_not_optimistic_and_only_matching_ack_completes_it(mut ctx: Ctx) {
    live(&mut ctx);
    let mut sh = shuffle(&[1, 2, 3]);
    publish(sh.clone());
    let mut view = PlanView::new();
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    let rows = view.rows.clone();
    view.move_selected(1, &ctx);
    let token = view.pending.as_ref().unwrap().0.clone();
    assert_eq!(view.rows, rows);
    view.move_selected(1, &ctx);
    assert_eq!(view.pending.as_ref().unwrap().0, token);
    sh.ack = Some(rormpc_player::PlanAck { token: "another-client".into(), ok: true, error: None });
    publish(sh.clone());
    view.refresh(&ctx, false);
    assert!(view.pending.is_some());
    sh.plan.swap(0, 1);
    sh.plan_version = "session:2".into();
    sh.ack.as_mut().unwrap().token = token;
    publish(sh);
    view.refresh(&ctx, false);
    assert!(view.pending.is_none());
    assert_eq!(view.selected_id, Some(1));
    assert_eq!(view.rows[0].id, Some(2));
}

#[rstest]
fn deadline_resync_accepts_published_ack_instead_of_timing_out(mut ctx: Ctx) {
    live(&mut ctx);
    let mut sh = shuffle(&[1, 2]);
    publish(sh.clone());
    let mut view = PlanView::new();
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    view.pending = Some(("ack".into(), Instant::now() - rormpc_player::ANSWER_TIMEOUT));
    sh.ack = Some(rormpc_player::PlanAck { token: "ack".into(), ok: true, error: None });
    publish(sh);
    view.refresh(&ctx, false);
    assert!(view.pending.is_none());
}

#[rstest]
fn stale_and_section_boundaries_never_submit_swaps(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 2]));
    let mut view = PlanView::new();
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    view.move_selected(-1, &ctx);
    assert!(view.pending.is_none());
    view.select_id(Some(2));
    view.refresh(&ctx, false);
    view.move_selected(1, &ctx);
    assert!(view.pending.is_none());
    view.select_id(Some(3));
    view.refresh(&ctx, false);
    view.move_selected(-1, &ctx);
    assert!(view.pending.is_none());
    ctx.player_present.store(false, std::sync::atomic::Ordering::Relaxed);
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    view.move_selected(1, &ctx);
    assert!(view.pending.is_none());
}

#[rstest]
fn toggle_sort_and_movement_do_not_change_physical_queue(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[4, 2, 3, 1]));
    let original = ctx.queue.clone();
    let mut pane = QueuePane::new(&ctx);
    pane.before_show(&ctx).unwrap();
    pane.handle_action(&mut queue(QueueActions::TogglePlanView), &mut ctx).unwrap();
    assert!(ctx.queue_plan.get());
    pane.handle_action(&mut queue(QueueActions::Shuffle), &mut ctx).unwrap();
    pane.handle_action(&mut common(CommonAction::MoveUp), &mut ctx).unwrap();
    pane.handle_action(&mut queue(QueueActions::TogglePlanView), &mut ctx).unwrap();
    assert!(!ctx.queue_plan.get());
    assert_eq!(
        ctx.queue.iter().map(|s| s.id).collect::<Vec<_>>(),
        original.iter().map(|s| s.id).collect::<Vec<_>>()
    );
}

#[rstest]
fn renderer_shows_plan_title_and_dimmed_stale_forecast(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 2]));
    ctx.player_present.store(false, std::sync::atomic::Ordering::Relaxed);
    let mut view = PlanView::new();
    let mut terminal = Terminal::new(TestBackend::new(120, 12)).unwrap();
    terminal
        .draw(|frame| {
            view.render(frame, frame.area(), &ctx);
        })
        .unwrap();
    let text = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect::<String>();
    assert!(text.contains("Queue · plan (o: queue) · stale"));
    assert!(text.contains("unplanned · queue order"));
    assert!(
        terminal.backend().buffer().content().iter().any(|c| c.modifier.contains(Modifier::DIM))
    );
}

#[rstest]
fn mouse_targets_projected_song_ids_after_recalculation(mut ctx: Ctx) {
    live(&mut ctx);
    let mut sh = shuffle(&[4, 2, 3, 1]);
    publish(sh.clone());
    let mut view = PlanView::new();
    view.area = Rect::new(0, 0, 80, 20);
    view.select_id(Some(2));
    view.refresh(&ctx, false);
    view.remember_rendered(&ctx);
    view.mouse(MouseEvent { x: 0, y: 0, kind: MouseEventKind::LeftClick }, &ctx).unwrap();
    assert_eq!(view.selected_id, Some(4));
    sh.plan.rotate_left(2);
    publish(sh);
    view.refresh(&ctx, false);
    view.remember_rendered(&ctx);
    view.mouse(MouseEvent { x: 0, y: 0, kind: MouseEventKind::LeftClick }, &ctx).unwrap();
    assert_eq!(view.selected_id, Some(3));
}

#[rstest]
fn every_header_sort_is_disabled_in_plan_mode(mut ctx: Ctx) {
    live(&mut ctx);
    ctx.queue_plan.set(true);
    let (tx, rx) = crossbeam::channel::unbounded();
    ctx.client_request_sender = tx;
    let formats: Vec<_> =
        ctx.config.theme.song_table_format.iter().map(|f| f.prop.clone()).collect();
    for i in 0..formats.len() {
        crate::ui::panes::queue_header::QueueHeaderPane::sort_by_column(&formats, i, &ctx).unwrap();
    }
    assert!(rx.try_recv().is_err());
}

#[rstest]
fn unreconciled_playing_id_is_stale_not_a_live_forecast(mut ctx: Ctx) {
    live(&mut ctx);
    ctx.status.songid = Some(1);
    publish(shuffle(&[1, 2]));
    let mut view = PlanView::new();
    view.refresh(&ctx, false);
    assert!(view.is_stale);
    assert_eq!(view.rows[0].turn, Turn::Current);
}

#[rstest]
fn missing_ack_deadline_never_retries_or_reorders(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 2]));
    let (tx, rx) = crossbeam::channel::unbounded();
    ctx.client_request_sender = tx;
    let mut view = PlanView::new();
    view.select_id(Some(1));
    view.refresh(&ctx, false);
    let rows = view.rows.clone();
    view.pending = Some(("missing".into(), Instant::now() - rormpc_player::ANSWER_TIMEOUT));
    view.refresh(&ctx, false);
    assert!(view.pending.is_none());
    assert_eq!(view.rows, rows);
    assert!(rx.try_recv().is_err());
}

#[rstest]
fn click_uses_last_painted_id_even_if_the_forecast_rebuilds_first(mut ctx: Ctx) {
    live(&mut ctx);
    let mut sh = shuffle(&[4, 2, 3, 1]);
    publish(sh.clone());
    let mut view = PlanView::new();
    view.area = Rect::new(0, 0, 80, 20);
    view.select_id(Some(2));
    view.refresh(&ctx, false);
    view.remember_rendered(&ctx);
    sh.plan.rotate_left(2);
    publish(sh);
    view.refresh(&ctx, false); // deliberately no paint yet
    assert_eq!(view.rows[0].id, Some(3));
    view.mouse(MouseEvent { x: 0, y: 0, kind: MouseEventKind::LeftClick }, &ctx).unwrap();
    assert_eq!(view.selected_id, Some(4)); // song 4 is what the user actually clicked
    view.remember_rendered(&ctx);
    ctx.queue[2].file = "reused-id".into();
    let selected = view.selected_id;
    view.mouse(MouseEvent { x: 0, y: 0, kind: MouseEventKind::DoubleClick }, &ctx).unwrap();
    assert_eq!(view.selected_id, selected); // same number, different file: no action on the reused ID
}

#[rstest]
fn removed_selection_cannot_play_or_delete_the_unpainted_fallback(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 2, 3]));
    let mut view = PlanView::new();
    view.select_id(Some(2));
    view.refresh(&ctx, false);
    view.remember_rendered(&ctx);
    let (tx, rx) = crossbeam::channel::unbounded();
    ctx.client_request_sender = tx;
    ctx.queue.retain(|s| s.id != 2);
    publish(shuffle(&[1, 3]));
    view.action(&mut queue(QueueActions::Play), &mut ctx).unwrap();
    view.action(&mut queue(QueueActions::Delete), &mut ctx).unwrap();
    assert_eq!(view.selected_id, Some(3));
    assert_eq!(view.selected_for_action(), None);
    assert!(rx.try_recv().is_err());
    view.remember_rendered(&ctx);
    assert_eq!(view.selected_for_action(), Some(3));
}

#[rstest]
fn reused_ids_drop_old_marks_and_block_action_until_paint(mut ctx: Ctx) {
    live(&mut ctx);
    publish(shuffle(&[1, 2, 3]));
    let mut view = PlanView::new();
    view.select_id(Some(2));
    view.marked.insert(2);
    view.refresh(&ctx, false);
    view.remember_rendered(&ctx);
    ctx.queue[1].file = "another-file".into();
    view.refresh(&ctx, false);
    assert!(!view.marked.contains(&2));
    assert_eq!(view.selected_for_action(), None);
}
