use std::collections::{HashMap, HashSet};

use anyhow::Result;
use enum_map::{Enum, EnumMap, enum_map};
use itertools::Itertools;
use ratatui::{
    Frame,
    layout::Flex,
    prelude::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Row, TableState},
};
use rmpc_mpd::{
    client::Client,
    commands::{IdleEvent, Song},
    mpd_client::{MpdClient, MpdCommand},
    proto_client::ProtoClient,
    queue_position::QueuePosition,
    single_or_range::SingleOrRange,
};

use super::Pane;
use crate::{
    MpdQueryResult,
    config::{
        keys::{
            CommonAction,
            GlobalAction,
            QueueActions,
            actions::{
                AddKind,
                AutoplayKind,
                CopyContent,
                CopyContentsKind,
                DeleteKind,
                RateKind,
                SaveKind,
                Sort,
                SortOpts,
            },
        },
        sort_mode::{SortMode, SortOptions},
        theme::{
            AlbumSeparator,
            properties::{Property, PropertyKindOrText, SongProperty},
        },
    },
    core::command::{create_env, run_external},
    ctx::{Ctx, LIKE_STICKER, RATING_STICKER},
    shared::{
        args,
        clipboard::Clipboard,
        events::AppEvent,
        ext::{btreeset_ranges::BTreeSetRanges, rect::RectExt},
        id::{self, Id},
        keys::ActionEvent,
        macros::{modal, status_error, status_info, status_warn},
        mouse_event::{MouseEvent, MouseEventKind, calculate_scrollbar_position},
        mpd_client_ext::{Enqueue, MpdClientExt},
        song_ext::SongsExt,
    },
    ui::{
        UiEvent,
        dirstack::{self, Dir, DirStackItem},
        input::{BufferId, InputResultEvent},
        modals::{
            confirm_modal::{Action, ConfirmModal},
            info_list_modal::{InfoListModal, SongCtx},
            input_modal::InputModal,
            menu::{
                add_to_playlist_or_show_modal,
                create_add_modal,
                create_copy_to_clipboard_modal,
                create_delete_modal,
                create_rating_modal,
                create_save_modal,
                delete_from_playlist_or_show_confirmation,
                modal::MenuModal,
            },
            select_modal::SelectModal,
        },
        panes::queue_header::QueueHeaderPane,
        song_ext::SongExt as _,
        widgets::virtualized_table::VirtualizedTable,
    },
};

#[derive(Debug)]
pub struct QueuePane {
    queue: Dir<Song, TableState>,
    plan: crate::ui::rormpc_queue_plan::PlanView,
    column_widths: Vec<Constraint>,
    column_formats: Vec<Property<SongProperty>>,
    areas: EnumMap<Areas, Rect>,
    should_center_cursor_on_current: bool,
    highlight_id: Id,
    highlight_enabled: bool,
    new_album_indices: HashSet<usize>,
    /// rormpc: the live filter (`/`); while set, `queue.items` holds only the matching songs, in queue order
    find: Option<QueueFind>,
    /// rormpc: the like column (x, width) of the last render, and the row whose like cell the mouse is over
    like_col: Option<(u16, u16)>,
    hover_like: Option<usize>,
    /// rormpc: Play's table follows the weighted shuffle instead of `o` (`follow_weighted`); None: `ctx.queue_plan`
    forced_plan: Option<bool>,
}

/// rormpc: state of the Queue's live filter.
#[derive(Debug)]
struct QueueFind {
    buffer: BufferId,
    /// keys go to the query (Enter plays, Esc restores)
    typing: bool,
    query: String,
    /// the song under the cursor and the scroll when filtering began, restored by Esc
    saved_id: Option<u32>,
    saved_offset: usize,
    /// no exact match: the rows shown are close matches (typos)
    close: bool,
}

#[derive(Debug, Enum)]
enum Areas {
    Table,
    Scrollbar,
    FilterArea,
}

const ADD_TO_PLAYLIST: &str = "add_to_playlist";
const ADD_TO_PLAYLIST_MULTIPLE: &str = "add_to_playlist_multiple";

impl QueuePane {
    pub fn new(ctx: &Ctx) -> Self {
        let (column_widths, column_formats) = Self::init(ctx);

        let mut s = Self {
            queue: Dir::new(ctx.queue.clone()),
            plan: crate::ui::rormpc_queue_plan::PlanView::new(),
            column_widths,
            column_formats,
            areas: enum_map! {
                _ => Rect::default(),
            },
            should_center_cursor_on_current: ctx.config.center_current_song_on_change,
            highlight_id: id::new(),
            highlight_enabled: true,
            new_album_indices: HashSet::new(),
            find: None,
            like_col: None,
            hover_like: None,
            forced_plan: None,
        };

        s.recalculate_album_indices();
        s.highlight_timeout(ctx);

        s
    }

    pub fn init(ctx: &Ctx) -> (Vec<Constraint>, Vec<Property<SongProperty>>) {
        (
            ctx.config
                .theme
                .song_table_format
                .iter()
                // This 0 is fine - song_table_format should never have the Ratio constraint
                .map(|v| v.width.into_constraint(0))
                .collect_vec(),
            ctx.config.theme.song_table_format.iter().map(|v| v.prop.clone()).collect_vec(),
        )
    }

    fn recalculate_album_indices(&mut self) {
        self.new_album_indices = self
            .queue
            .items
            .as_slice()
            .to_album_ranges()
            .map(|range| range.end.saturating_sub(1))
            .collect();
    }

    /// rormpc: point the physical-order Dir at the plan view's selected and marked song IDs, so the
    /// ordinary ID-based Queue actions act on the songs the plan view shows.
    /// rormpc: the plan view is shown (Play: weighted on; elsewhere: `o`).
    fn plan_view(&self, ctx: &Ctx) -> bool {
        self.forced_plan.unwrap_or_else(|| ctx.queue_plan.get())
    }

    /// rormpc: Play's table: the plan view while the weighted shuffle is on, MPD's queue order while it is off.
    /// It never touches `ctx.queue_plan`, so a Queue pane elsewhere keeps its own `o` state.
    pub(crate) fn follow_weighted(&mut self, on: bool, ctx: &Ctx) {
        self.plan.name = "Plan (w off: queue order)";
        if self.forced_plan != Some(on) {
            self.forced_plan = Some(!on); // switch_view goes from the view shown to the other one
            self.switch_view(ctx);
        }
    }

    /// rormpc: the song under the cursor, in either view.
    pub(crate) fn selected_song(&self, ctx: &Ctx) -> Option<Song> {
        if self.plan_view(ctx) {
            let id = self.plan.selected_for_action()?;
            return ctx.queue.iter().find(|s| s.id == id).cloned();
        }
        self.queue.selected().cloned()
    }

    /// rormpc: Esc has something to do in the table itself (marks, the `/` filter, the plan's search).
    pub(crate) fn esc_pending(&self, ctx: &Ctx) -> bool {
        if self.plan_view(ctx) {
            return !self.plan.marked.is_empty() || self.plan.typing || self.plan.has_query();
        }
        !self.queue.marked().is_empty() || self.find.is_some()
    }

    /// rormpc: physical queue order <-> plan view, keeping the selected and marked songs by ID.
    fn switch_view(&mut self, ctx: &Ctx) {
        let to_plan = !self.plan_view(ctx);
        if to_plan {
            let selected = self.queue.selected().map(|s| s.id);
            let marked = self
                .queue
                .marked()
                .iter()
                .filter_map(|i| self.queue.items.get(*i))
                .map(|s| s.id)
                .collect();
            if self.find.is_some() {
                self.end_find(ctx, false);
            }
            self.plan.select_id(selected);
            self.plan.marked = marked;
        } else {
            let selected = self.plan.selected_id;
            self.queue.items.clone_from(&ctx.queue);
            let idx = selected.and_then(|id| ctx.queue.iter().position(|s| s.id == id));
            self.queue.select_idx(idx.unwrap_or(0), ctx.config.scrolloff);
            self.map_plan_marks(ctx);
            self.recalculate_album_indices();
        }
        match self.forced_plan.as_mut() {
            Some(forced) => *forced = to_plan,
            None => ctx.queue_plan.set(to_plan),
        }
        if to_plan {
            self.plan.refresh(ctx, false);
            crate::ui::rormpc_player::refresh_presence(ctx);
        }
    }

    fn map_plan_selection(&mut self, ctx: &Ctx) {
        self.queue.items.clone_from(&ctx.queue);
        self.queue.state.set_content_len(Some(ctx.queue.len()));
        let selected = self
            .plan
            .selected_for_action()
            .and_then(|id| ctx.queue.iter().position(|s| s.id == id));
        self.queue.state.select(selected, ctx.config.scrolloff);
        self.map_plan_marks(ctx);
    }

    fn map_plan_marks(&mut self, ctx: &Ctx) {
        *self.queue.marked_mut() = ctx
            .queue
            .iter()
            .enumerate()
            .filter(|(_, s)| self.plan.marked.contains(&s.id))
            .map(|(i, _)| i)
            .collect();
    }

    fn highlight_timeout(&mut self, ctx: &Ctx) {
        if let Some(delay) = ctx.config.queue_disable_current_item_style_timeout_ms {
            self.highlight_enabled = true;
            ctx.scheduler.schedule_replace(self.highlight_id, delay, |(tx, _)| {
                tx.send(AppEvent::UiEvent(UiEvent::DisableQueueHighlight))?;
                Ok(())
            });
        }
    }

    fn enqueue_items(&self, all: bool) -> (Vec<Enqueue>, Option<usize>) {
        let hovered = self.queue.selected().map(|s| s.file.as_str());
        self.items(all).fold((Vec::new(), None), |mut acc, (idx, song)| {
            let path = song.file.clone();
            if hovered.as_ref().is_some_and(|hovered| hovered == &path) {
                acc.1 = Some(idx);
            }

            acc.0.push(Enqueue::File { path });

            acc
        })
    }

    fn items<'a>(&'a self, all: bool) -> Box<dyn Iterator<Item = (usize, &'a Song)> + 'a> {
        if all {
            Box::new(self.queue.items.iter().enumerate())
        } else if self.queue.marked().is_empty() {
            if let Some((idx, item)) = self.queue.selected_with_idx() {
                Box::new(std::iter::once((idx, item)))
            } else {
                Box::new(std::iter::empty::<(usize, &Song)>())
            }
        } else {
            Box::new(self.queue.marked().iter().map(|idx| (*idx, &self.queue.items[*idx])))
        }
    }

    fn open_context_menu(&mut self, ctx: &Ctx) {
        let selected_song = self.queue.selected().cloned();
        let selected_song_id = selected_song.as_ref().map(|s| s.id);

        let modal = MenuModal::new(ctx)
            .list_section(ctx, |mut section| {
                section.add_item("Play", move |ctx| {
                    if let Some(id) = selected_song_id {
                        ctx.command(move |_, client| {
                            client.play_id(id)?;
                            Ok(())
                        });
                    }
                    Ok(())
                });
                section.add_item("Show info", move |ctx| {
                    if let Some(song) = selected_song {
                        modal!(
                            ctx,
                            InfoListModal::builder()
                                .items(SongCtx(&song, ctx))
                                .title("Song info")
                                .column_widths(&[30, 70])
                                .build()
                        );
                    }
                    Ok(())
                });
                // rormpc: the playing song's listen, same as oL (only where a scrobbler writes its status)
                if selected_song_id.is_some()
                    && selected_song_id == ctx.status.songid
                    && crate::ui::rormpc_scrobble::status().is_some()
                {
                    let hint = crate::ui::rormpc_scrobble::key_hint(ctx);
                    section.add_item(format!("Send to ListenBrainz now{hint}"), |ctx| {
                        crate::ui::rormpc_scrobble::send_now(ctx);
                        Ok(())
                    });
                }
                Some(section)
            })
            .list_section(ctx, |mut section| {
                let items = self.queue.items.iter().map(|song| song.file.clone()).collect_vec();
                section.add_item("Add queue to playlist", |ctx| {
                    let playlists = ctx.query_sync(move |client| {
                        Ok(client.list_playlists()?.into_iter().map(|p| p.name).collect_vec())
                    })?;

                    modal!(
                        ctx,
                        SelectModal::builder()
                            .ctx(ctx)
                            .options(playlists)
                            .confirm_label("Add")
                            .title("Select a playlist")
                            .on_confirm(move |ctx, selected, _idx| {
                                ctx.command(move |_, client| {
                                    client.add_to_playlist_multiple(&selected, items)?;
                                    Ok(())
                                });
                                Ok(())
                            })
                            .build()
                    );
                    Ok(())
                });
                section.add_item("Save queue as playlist", move |ctx| {
                    modal!(
                        ctx,
                        InputModal::new(ctx)
                            .title("Create new playlist")
                            .confirm_label("Save")
                            .input_label("Playlist name:")
                            .on_confirm(move |ctx, value| {
                                let value = value.to_owned();
                                ctx.command(move |_, client| {
                                    client.save_queue_as_playlist(&value, None)?;
                                    Ok(())
                                });
                                Ok(())
                            })
                    );
                    Ok(())
                });

                Some(section)
            })
            // rormpc: like / dislike the song, kept away from the destructive items
            .list_section(ctx, |mut section| {
                let file = self.queue.selected().map(|s| s.file.clone())?;
                let hint = crate::ui::rormpc_actions::rate_key_hint(ctx);
                for (label, value) in [("Like ♥", "2"), ("Dislike ✗", "0"), ("Clear like", "1")] {
                    let file = file.clone();
                    section.add_item(format!("{label}{hint}"), move |ctx| {
                        crate::ui::rormpc_actions::set_like(ctx, file, value);
                        Ok(())
                    });
                }
                // rormpc: a "Not finished" deletion candidate can be kept
                if ctx.song_stickers(&file).is_some_and(|st| st.contains_key("notFinished")) {
                    let keep_file = file.clone();
                    section.add_item("Keep (drop from Not finished)", move |_| {
                        crate::ui::rormpc_actions::keep_song(keep_file);
                        Ok(())
                    });
                }
                // marked rows, else the cursor row
                let targets: Vec<String> = if self.queue.marked().is_empty() {
                    vec![file.clone()]
                } else {
                    self.queue.marked().iter().filter_map(|i| self.queue.items.get(*i)).map(|s| s.file.clone()).collect()
                };
                let what = if targets.len() == 1 {
                    self.queue.selected().filter(|_| self.queue.marked().is_empty())
                        .map(|s| format!("'{}'", s.metadata.get("title").map_or(s.file.as_str(), |t| t.last())))
                        .unwrap_or_else(|| "1 song".to_owned())
                } else {
                    format!("{} marked songs", targets.len())
                };
                let next_targets = targets.clone();
                section.add_item("Play next", move |ctx| {
                    crate::ui::rormpc_upnext::play_next(ctx, next_targets);
                    Ok(())
                });
                section.add_item("Add to playlist…", move |ctx| {
                    crate::ui::rormpc_playlists::open_add_to_playlist(ctx, targets, what);
                    Ok(())
                });
                // weighted shuffle (mpd-player): rest the song, or bring it back
                let shuffle_file = file.clone();
                let shuffle_title = self.queue.selected().map(|s| s.metadata.get("title").map_or(s.file.as_str(), |t| t.last()).to_owned()).unwrap_or_default();
                let genre_what = format!("'{shuffle_title}'");
                if crate::ui::rormpc_player::cooldown_days(&shuffle_file).is_some() {
                    section.add_item("Back in the weighted shuffle (undo heard enough)", move |ctx| {
                        crate::ui::rormpc_player::unheard_enough(ctx, shuffle_file.clone());
                        Ok(())
                    });
                } else {
                    section.add_item("Heard enough (rests in the weighted shuffle, e)", move |ctx| {
                        crate::ui::rormpc_player::heard_enough(ctx, shuffle_file.clone(), shuffle_title.clone());
                        Ok(())
                    });
                }
                let tags_file = file.clone();
                section.add_item("Tags…", move |ctx| {
                    crate::ui::rormpc_tags::open_tags_menu(ctx, tags_file);
                    Ok(())
                });
                let genre_file = file.clone();
                section.add_item("Pin genre in Hits…", move |ctx| {
                    crate::ui::rormpc_genres::open_pin_menu_for_file(ctx, genre_file, genre_what);
                    Ok(())
                });
                if let Some(song) = self.queue.selected().cloned() {
                    use crate::ui::rormpc_exceptions::{Kind, key_hint, open_for_queue_song};
                    let pin_song = song.clone();
                    section.add_item(format!("Pin in Hits results…{}", key_hint(ctx, Kind::Pin)), move |ctx| {
                        open_for_queue_song(ctx, Kind::Pin, &pin_song);
                        Ok(())
                    });
                    section.add_item(format!("Exclude from Hits results…{}", key_hint(ctx, Kind::Exclude)), move |ctx| {
                        open_for_queue_song(ctx, Kind::Exclude, &song);
                        Ok(())
                    });
                }
                let lyrics_file = file.clone();
                section.add_item("Choose lyrics…", move |ctx| {
                    crate::ui::rormpc_lyrics::open_chooser(ctx, lyrics_file);
                    Ok(())
                });
                Some(section)
            })
            // rormpc: playback (mpd-player) and sources: also with an empty queue, e.g. right after "Clear queue"
            .list_section(ctx, |mut section| {
                let on = crate::ui::rormpc_player::shuffle_state().enabled;
                section.add_item(format!("Weighted shuffle: {} (w)", if on { "on → off" } else { "off → on" }), |ctx| {
                    crate::ui::rormpc_player::toggle_shuffle(ctx);
                    Ok(())
                });
                let gap = crate::ui::rormpc_player::gap_seconds()
                    .map_or_else(String::new, |g| if g > 0.0 { format!(" (now {g} s)") } else { " (now off)".to_owned() });
                section.add_item(format!("Silence between songs…{gap}"), |ctx| {
                    crate::ui::rormpc_player::open_gap_menu(ctx);
                    Ok(())
                });
                let pause = crate::ui::rormpc_pause::remaining().map_or_else(
                    || "Pause for…".to_owned(),
                    |left| format!("Paused, plays on in {}…", crate::ui::rormpc_pause::fmt_remaining(left)),
                );
                section.add_item(pause, |ctx| {
                    crate::ui::rormpc_pause::open_pause_menu(ctx);
                    Ok(())
                });
                section.add_item("Sources… (play the library or a playlist)", |ctx| {
                    crate::ui::rormpc_upnext::open_sources(ctx);
                    Ok(())
                });
                Some(section)
            })
            // rormpc: the other library files with this song's name (Versions)
            .list_section(ctx, |mut section| {
                let song = self.queue.selected()?;
                let (file, view) = (song.file.clone(), (song.id, self.queue.state.offset()));
                if !crate::ui::rormpc_versions::has_versions(&file) {
                    return None;
                }
                section.add_item(format!("Find versions…{}", crate::ui::rormpc_versions::key_hint(ctx)), move |ctx| {
                    crate::ui::rormpc_versions::find_versions(ctx, &file, Some(view));
                    Ok(())
                });
                Some(section)
            })
            // rormpc: deleting the file is its own section: the same delete menu as Ctrl-x
            .list_section(ctx, |mut section| {
                let file = self.queue.selected().map(|s| s.file.clone())?;
                let hint = crate::ui::rormpc_actions::external_key_hint(ctx, &["musicdb", "delete"]);
                section.add_item(format!("Delete library file…{hint}"), move |ctx| {
                    crate::ui::rormpc_actions::open_delete_menu(ctx, vec![file]);
                    Ok(())
                });
                Some(section)
            })
            .list_section(ctx, |section| {
                let section = section
                    .item("Remove from queue (keep file)", move |ctx| {
                        if let Some(id) = selected_song_id {
                            ctx.command(move |_, client| {
                                client.delete_id(id)?;
                                Ok(())
                            });
                        }
                        Ok(())
                    })
                    // rormpc: the same file queued several times (e.g. from Hits) collapses to one entry
                    .item(
                        match crate::ui::rormpc_actions::duplicate_ids(&ctx.queue, ctx.current_song().map(|s| s.id))
                            .len()
                        {
                            0 => "Remove repeated entries (none)".to_owned(),
                            n => format!("Remove repeated entries ({n})…"),
                        },
                        |ctx| {
                            let current = ctx.current_song().map(|s| s.id);
                            let n = crate::ui::rormpc_actions::duplicate_ids(&ctx.queue, current).len();
                            if n > 0 {
                                crate::ui::rormpc_actions::confirm_remove_duplicates(ctx, n);
                            }
                            Ok(())
                        },
                    )
                    // rormpc: asks first: a menu item runs on one click, so a stray click wiped the queue
                    .item("Clear queue…", |ctx| {
                        crate::ui::rormpc_actions::confirm_clear_queue(ctx);
                        Ok(())
                    });
                Some(section)
            })
            .list_section(ctx, |section| {
                let section = section.item("Cancel", |_ctx| Ok(()));
                Some(section)
            })
            .build();

        modal!(ctx, modal);
    }

    fn sort(opts: SortOpts, ctx: &Ctx) -> Result<()> {
        let opts = SortOptions {
            mode: SortMode::Format(opts.tags),
            group_by_type: false,
            reverse: opts.descending,
            ignore_leading_the: false,
            fold_case: true,
        };

        let mut evald = ctx
            .queue
            .iter()
            .map(|song| (song.id, song))
            .sorted_by(|a, b| a.1.with_custom_sort(&opts).cmp(&b.1.with_custom_sort(&opts)))
            .collect_vec();

        if ctx.queue.iter().map(|song| (song.id, song)).zip(evald.iter()).all(|(a, b)| a.0 == b.0) {
            evald.reverse();
        }

        let swaps = QueueHeaderPane::calculate_swaps(evald.as_slice(), ctx)?;

        ctx.command(move |_, client| {
            client.send_start_cmd_list()?;
            for swap in swaps {
                client.send_swap_position(swap.0, swap.1)?;
            }
            client.send_execute_cmd_list()?;
            client.read_ok()?;
            Ok(())
        });

        Ok(())
    }
}

impl Pane for QueuePane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> anyhow::Result<()> {
        if self.plan_view(ctx) {
            self.plan.render(frame, area, ctx);
            return Ok(());
        }
        let Ctx { config, .. } = ctx;
        self.calculate_areas(area, ctx)?;

        let filter_text = self.queue.filter_text(self.areas[Areas::Table].width, ctx);

        let table_block = {
            let border_style = config.as_border_style();
            let mut b = Block::default().border_style(border_style);
            if self.areas[Areas::FilterArea].height == 0
                && let Some(ref title) = filter_text
            {
                b = b.title(title.clone());
            }
            b
        };

        self.queue.state.set_content_and_viewport_len(
            self.queue.len(),
            self.areas[Areas::Table].height as usize,
        );

        let widths = Layout::horizontal(self.column_widths.as_slice())
            .flex(Flex::Start)
            .spacing(1)
            .split(self.areas[Areas::Table]);

        let formats = &config.theme.song_table_format;
        // rormpc: the Versions column (≋) reads its cache once, in the background
        if formats.iter().any(|f| matches!(f.prop.kind, PropertyKindOrText::Property(SongProperty::Versions()))) {
            crate::ui::rormpc_versions::ensure_loaded(&ctx.app_event_sender);
        }
        // rormpc: the column showing rmpc's like sticker gets a clickable heart (hover shows ♡ on an unrated song)
        let like_idx = formats.iter().position(|f| format!("{:?}", f.prop).contains("Sticker(\"like\")"));
        let has_next_col = formats.iter().any(|f| format!("{:?}", f.prop).contains("ShuffleNext"));
        self.like_col = like_idx.and_then(|i| widths.get(i)).map(|r| (r.x, r.width.max(1)));
        let hover_like = self.hover_like;

        let marker_symbol_len = config.theme.symbols.marker.chars().count();
        // rormpc: a file queued more than once gets a dim badge in the first column
        let duplicates: HashSet<String> = {
            let mut copies: HashMap<&str, usize> = HashMap::new();
            for song in &self.queue.items {
                *copies.entry(song.file.as_str()).or_default() += 1;
            }
            copies.into_iter().filter(|(_, n)| *n > 1).map(|(f, _)| f.to_owned()).collect()
        };
        const DUPLICATE_BADGE: &str = "⧉ ";
        // rormpc: Up next entries show their turn ("↑1 ")
        let up_next = crate::ui::rormpc_upnext::up_next_ids();

        let current_song_id = ctx.current_song().map(|s| s.id);
        let marked = std::mem::take(self.queue.marked_mut());
        let filter = ctx.input.value(self.queue.filter_buffer_id);
        let selected_idx = self.queue.selected_idx();

        let table = VirtualizedTable::new(&self.queue.items)
            .column_widths(self.column_widths.clone())
            .map_fn(|idx, song| {
                let is_currently_playing_song = current_song_id.is_some_and(|v| v == song.id);
                let is_under_cursor = selected_idx.is_some_and(|i| i == idx);

                let is_marked = marked.contains(&idx);
                let is_duplicate = duplicates.contains(&song.file);
                // the badge only when the theme has no ShuffleNext column, which shows the same turn
                let up_badge = (!has_next_col)
                    .then(|| up_next.iter().position(|id| *id == song.id).map(|k| format!("↑{} ", k + 1)))
                    .flatten();
                // a pin ✚ or an exclusion ⊘ (`+` / `-`) names this file
                let except_badge =
                    crate::ui::rormpc_exceptions::mark_for(ctx, &song.file).map(|k| format!("{} ", k.mark()));
                let matches_filter = is_currently_playing_song
                    || if self.queue.filter_active {
                        song.matches_formats(self.column_formats.as_slice(), &filter, ctx)
                    } else {
                        Default::default()
                    };

                let columns = (0..formats.len()).map(|i| {
                    let mut max_len: usize = widths[i].width.into();
                    // We have to subtract marker symbol length from max len in
                    // order to make space for the marker
                    // symbol in case we are in the first column of the table
                    // and the song is marked.
                    if is_marked && i == 0 {
                        max_len = max_len.saturating_sub(marker_symbol_len);
                    }
                    if is_duplicate && i == 0 {
                        max_len = max_len.saturating_sub(DUPLICATE_BADGE.chars().count());
                    }
                    if let (Some(badge), 0) = (&up_badge, i) {
                        max_len = max_len.saturating_sub(badge.chars().count());
                    }
                    if let (Some(badge), 0) = (&except_badge, i) {
                        max_len = max_len.saturating_sub(badge.chars().count());
                    }
                    let format = &formats[i];

                    let mut line = if let Some(speed) = format.scroll_speed {
                        song.as_line_scrolling(
                            &format.prop,
                            max_len,
                            &config.theme.format_tag_separator,
                            config.theme.multiple_tag_resolution_strategy,
                            speed,
                            ctx,
                        )
                    } else {
                        song.as_line_ellipsized(
                            &format.prop,
                            max_len,
                            &config.theme.symbols,
                            &config.theme.format_tag_separator,
                            config.theme.multiple_tag_resolution_strategy,
                            ctx,
                        )
                    }
                    .unwrap_or_default()
                    .alignment(formats[i].alignment.into());

                    if is_marked && i == 0 {
                        let marker_style =
                            dirstack::marker_style(ctx, is_under_cursor, matches_filter);
                        let marker_span = Span::styled(&config.theme.symbols.marker, marker_style);

                        line.spans.splice(..0, std::iter::once(marker_span));
                    }
                    if is_duplicate && i == 0 {
                        let badge = Span::styled(DUPLICATE_BADGE, Style::default().add_modifier(Modifier::DIM));
                        line.spans.insert(usize::from(is_marked), badge);
                    }
                    if let (Some(badge), 0) = (&up_badge, i) {
                        let badge = Span::styled(badge.clone(), Style::default().add_modifier(Modifier::BOLD));
                        line.spans.insert(usize::from(is_marked), badge);
                    }
                    if let (Some(badge), 0) = (&except_badge, i) {
                        line.spans.insert(usize::from(is_marked), Span::raw(badge.clone()));
                    }
                    if Some(i) == like_idx && hover_like == Some(idx) && line.width() == 0 {
                        // the liked glyph, dimmed: same shape, so it reads "click to like" (♡ is narrower in some fonts)
                        line = Line::from(Span::styled("♥", Style::default().add_modifier(Modifier::DIM)));
                    }

                    line
                });

                let mut row = QueueRow::default();
                if matches_filter {
                    row.cell_style = Some(config.theme.highlighted_item_style);
                }
                if is_under_cursor && self.highlight_enabled {
                    row.cursor_style = Some(config.theme.current_item_style);
                }

                let sep = ctx.config.theme.song_table_album_separator;
                if self.new_album_indices.contains(&idx)
                    && matches!(sep, AlbumSeparator::Underline)
                    && idx != self.queue.items.len().saturating_sub(1)
                {
                    row.underlined = true;
                }

                row.into_row(columns)
            });

        frame.render_widget(table_block, self.areas[Areas::Table]);

        frame.render_stateful_widget(table, self.areas[Areas::Table], &mut self.queue.state);

        let _ = std::mem::replace(self.queue.marked_mut(), marked);

        if let Some(scrollbar) = config.as_styled_scrollbar()
            && self.areas[Areas::Scrollbar].width > 0
        {
            frame.render_stateful_widget(
                scrollbar,
                self.areas[Areas::Scrollbar],
                self.queue.state.as_scrollbar_state_ref(),
            );
        }

        if let Some(f) = &self.find {
            self.render_find_line(frame, f, ctx);
        } else if let Some(filter_text) = filter_text
            && self.areas[Areas::FilterArea].height > 0
        {
            frame.render_widget(
                Line::from(filter_text).style(
                    config.theme.text_color.map(|c| Style::default().fg(c)).unwrap_or_default(),
                ),
                self.areas[Areas::FilterArea],
            );
        }

        Ok(())
    }

    fn calculate_areas(&mut self, area: Rect, ctx: &Ctx) -> Result<()> {
        let Ctx { config, .. } = ctx;

        let scrollbar_area_width: u16 = config.theme.scrollbar.is_some().into();

        let [table_area, scrollbar_area] = Layout::horizontal([
            Constraint::Percentage(100),
            Constraint::Length(scrollbar_area_width),
        ])
        .areas(area);

        let mut table_area = if self.queue.filter_active || self.find.is_some() {
            self.areas[Areas::FilterArea] =
                Rect::new(table_area.x, table_area.y, table_area.width, 1);
            table_area.shrink_from_top(1)
        } else {
            self.areas[Areas::FilterArea] = Rect::default();
            table_area
        };

        // Create 1 column space between the table and the scrollbar
        table_area.width = table_area.width.saturating_sub(1);

        self.areas[Areas::Table] = table_area;
        self.areas[Areas::Scrollbar] = scrollbar_area;

        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.queue.state.set_content_and_viewport_len(
            self.queue.len(),
            self.areas[Areas::Table].height as usize,
        );

        // rormpc: back from "Find versions…": the same row and scroll, by song id (positions may have moved)
        let back = crate::ui::rormpc_versions::take_queue_view().and_then(|(id, offset)| {
            self.queue.items.iter().position(|s| s.id == id).map(|idx| (idx, offset))
        });
        if let Some((idx, offset)) = back {
            self.queue.select_idx(idx, ctx.config.scrolloff);
            self.queue.state.set_offset(offset);
        } else if self.should_center_cursor_on_current && self.find.is_none() {
            let to_select = ctx.current_song_index().or(self.queue.selected_idx()).or(Some(0));
            self.queue.select_idx_opt(to_select, usize::MAX);
            self.should_center_cursor_on_current = false;
        } else {
            let to_select = self.queue.selected_idx().or(ctx.current_song_index()).or(Some(0));
            self.queue.select_idx_opt(to_select, usize::MAX);
        }

        self.highlight_timeout(ctx);

        Ok(())
    }

    fn resize(&mut self, _area: Rect, ctx: &Ctx) -> Result<()> {
        self.queue.state.set_content_and_viewport_len(
            self.queue.len(),
            self.areas[Areas::Table].height as usize,
        );
        let to_select = self.queue.selected_idx().or(ctx.current_song_index()).or(Some(0));
        self.queue.select_idx_opt(to_select, ctx.config.scrolloff);
        ctx.render()?;
        Ok(())
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        match event {
            UiEvent::Database | UiEvent::QueueChanged if self.find.is_some() => {
                self.apply_find(ctx, false); // the filtered view is rebuilt from MPD's queue, by song id
            }
            UiEvent::SongChanged if self.find.is_some() => {} // a song change never moves the filtered cursor
            UiEvent::Database => {
                crate::ui::rormpc_versions::changed(&ctx.app_event_sender); // a deletion or download changes groups
                self.queue.filter_active = false;
                self.queue.items.clone_from(&ctx.queue);
                self.queue.unmark_all();
                self.recalculate_album_indices();
            }
            UiEvent::QueueChanged => {
                let marked = crate::ui::rormpc_actions::remap_marks(
                    &self.queue.items,
                    self.queue.marked(),
                    &ctx.queue,
                );
                self.queue.items.clone_from(&ctx.queue);
                *self.queue.marked_mut() = marked;
                self.recalculate_album_indices();
            }
            UiEvent::SongChanged => {
                if let Some(idx) = ctx.current_song_index()
                    && ctx.config.select_current_song_on_change
                {
                    match (is_visible, ctx.config.center_current_song_on_change) {
                        (true, true) => {
                            self.queue.select_idx(idx, usize::MAX);
                        }
                        (false, true) => {
                            self.queue.select_idx(idx, usize::MAX);
                            self.should_center_cursor_on_current = true;
                        }
                        (true, false) | (false, false) => {
                            self.queue.select_idx(idx, ctx.config.scrolloff);
                        }
                    }

                    ctx.render()?;
                }
            }
            UiEvent::Reconnected => {
                crate::ui::rormpc_player::refresh_presence(ctx);
                self.before_show(ctx)?;
                self.recalculate_album_indices();
            }
            UiEvent::ConfigChanged => {
                let (column_widths, column_formats) = Self::init(ctx);
                self.column_formats = column_formats;
                self.column_widths = column_widths;
            }
            UiEvent::DisableQueueHighlight => {
                self.highlight_enabled = false;
                ctx.render()?;
            }
            _ => {}
        }

        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        if self.plan_view(ctx) {
            if !self.plan.mouse(event, ctx)? {
                self.map_plan_selection(ctx);
                self.open_context_menu(ctx);
            }
            return Ok(());
        }
        let position = event.into();

        // rormpc: the like cell. Hover shows a heart; a click toggles like <-> no rating and neither selects nor plays
        let table = self.areas[Areas::Table];
        let on_like = self.like_col.is_some_and(|(x, w)| event.x >= x && event.x < x + w) && table.contains(position);
        let like_row = || self.queue.state.get_at_rendered_row(event.y.saturating_sub(table.y).into());
        if matches!(event.kind, MouseEventKind::Moved) {
            let hover = if on_like { like_row() } else { None };
            if hover != self.hover_like {
                self.hover_like = hover;
                ctx.render()?;
            }
            return Ok(());
        }
        if on_like && matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::DoubleClick) {
            if let Some(song) = like_row().and_then(|i| self.queue.items.get(i)) {
                let liked = ctx.song_stickers(&song.file).and_then(|st| st.get("like")).is_some_and(|v| v == "2");
                crate::ui::rormpc_actions::set_like(ctx, song.file.clone(), if liked { "1" } else { "2" });
                let title = song.metadata.get("title").map_or(song.file.as_str(), |t| t.last()).to_owned();
                status_info!("{}: {title}", if liked { "Like removed" } else { "Liked ♥" });
            }
            return Ok(());
        }

        if let Some(scrollbar_area) = self.scrollbar_area()
            && ctx.config.theme.scrollbar.is_some()
            && matches!(event.kind, MouseEventKind::LeftClick | MouseEventKind::Drag { .. })
            && let Some(perc) = calculate_scrollbar_position(event, scrollbar_area)
        {
            self.queue.state.scroll_to(perc, ctx.config.scrolloff);
            ctx.render()?;
            return Ok(());
        }

        if !self.areas[Areas::Table].contains(position) {
            return Ok(());
        }

        match event.kind {
            MouseEventKind::LeftClick if self.areas[Areas::Table].contains(event.into()) => {
                let clicked_row: usize = event.y.saturating_sub(self.areas[Areas::Table].y).into();
                if let Some(idx) = self.queue.state.get_at_rendered_row(clicked_row) {
                    self.queue.select_idx(idx, ctx.config.scrolloff);

                    ctx.render()?;
                }
            }
            MouseEventKind::LeftClick => {}
            MouseEventKind::DoubleClick if self.areas[Areas::Table].contains(event.into()) => {
                let clicked_row: usize = event.y.saturating_sub(self.areas[Areas::Table].y).into();

                if let Some(song) = self
                    .queue
                    .state
                    .get_at_rendered_row(clicked_row)
                    .and_then(|idx| self.queue.items.get(idx))
                {
                    let id = song.id;
                    ctx.command(move |_, client| {
                        client.play_id(id)?;
                        Ok(())
                    });
                }
            }
            MouseEventKind::DoubleClick => {}
            MouseEventKind::MiddleClick if self.areas[Areas::Table].contains(event.into()) => {
                let clicked_row: usize = event.y.saturating_sub(self.areas[Areas::Table].y).into();

                if let Some(selected_song) = self
                    .queue
                    .state
                    .get_at_rendered_row(clicked_row)
                    .and_then(|idx| self.queue.items.get(idx))
                {
                    let id = selected_song.id;
                    ctx.command(move |_, client| {
                        client.delete_id(id)?;
                        Ok(())
                    });
                }
            }
            MouseEventKind::MiddleClick => {}
            MouseEventKind::ScrollDown if self.areas[Areas::Table].contains(event.into()) => {
                self.queue.scroll_down(ctx.config.scroll_amount, ctx.config.scrolloff);
                ctx.render()?;
            }
            MouseEventKind::ScrollDown => {}
            MouseEventKind::ScrollUp if self.areas[Areas::Table].contains(event.into()) => {
                self.queue.scroll_up(ctx.config.scroll_amount, ctx.config.scrolloff);
                ctx.render()?;
            }
            MouseEventKind::ScrollUp => {}
            MouseEventKind::RightClick if self.areas[Areas::Table].contains(event.into()) => {
                let clicked_row: usize = event.y.saturating_sub(self.areas[Areas::Table].y).into();
                if let Some(idx) = self.queue.state.get_at_rendered_row(clicked_row) {
                    self.queue.select_idx(idx, ctx.config.scrolloff);

                    ctx.render()?;
                }
                if self.find.is_none() {
                    self.open_context_menu(ctx); // its moves act on positions
                }
            }
            MouseEventKind::RightClick => {}
            MouseEventKind::Drag { .. } => {}
            MouseEventKind::Moved => {}
        }

        self.highlight_timeout(ctx);

        Ok(())
    }

    fn on_query_finished(
        &mut self,
        id: &'static str,
        data: MpdQueryResult,
        _is_visible: bool,
        ctx: &Ctx,
    ) -> Result<()> {
        match (id, data) {
            (ADD_TO_PLAYLIST, MpdQueryResult::AddToPlaylist { playlists, song_file }) => {
                modal!(
                    ctx,
                    SelectModal::builder()
                        .ctx(ctx)
                        .options(playlists)
                        .confirm_label("Add")
                        .title("Select a playlist")
                        .on_confirm(move |ctx, selected, _idx| {
                            let song_file = song_file.clone();
                            ctx.command(move |_, client| {
                                if song_file.starts_with('/') {
                                    client.add_to_playlist(
                                        &selected,
                                        &format!("file://{song_file}"),
                                        None,
                                    )?;
                                } else {
                                    client.add_to_playlist(&selected, &song_file, None)?;
                                }
                                status_info!("Song added to playlist {}", selected);
                                Ok(())
                            });
                            Ok(())
                        })
                        .build()
                );
            }
            (
                ADD_TO_PLAYLIST_MULTIPLE,
                MpdQueryResult::AddToPlaylistMultiple { playlists, song_files },
            ) => {
                modal!(
                    ctx,
                    SelectModal::builder()
                        .ctx(ctx)
                        .options(playlists)
                        .confirm_label("Add")
                        .title("Select a playlist")
                        .on_confirm(move |ctx, selected, _idx| {
                            ctx.command(move |_, client| {
                                let songs_len = song_files.len();
                                for song_file in song_files {
                                    if song_file.starts_with('/') {
                                        client.add_to_playlist(
                                            &selected,
                                            &format!("file://{song_file}"),
                                            None,
                                        )?;
                                    } else {
                                        client.add_to_playlist(&selected, &song_file, None)?;
                                    }
                                }
                                status_info!("{} songs added to playlist {}", songs_len, selected);
                                Ok(())
                            });
                            Ok(())
                        })
                        .build()
                );
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &mut Ctx) -> Result<()> {
        if self.plan_view(ctx) {
            return self.plan.insert(&kind, ctx);
        }
        if let Some(f) = self.find.as_mut().filter(|f| f.typing) {
            match kind {
                InputResultEvent::Push | InputResultEvent::Pop => {
                    f.query = ctx.input.value(f.buffer);
                    self.apply_find(ctx, true);
                }
                InputResultEvent::Confirm => self.end_find(ctx, true),
                InputResultEvent::Cancel => self.end_find(ctx, false),
                InputResultEvent::NoChange => {}
            }
            ctx.render()?;
            return Ok(());
        }
        match kind {
            InputResultEvent::Push => {
                self.queue.recalculate_matched_items(self.column_formats.as_slice(), ctx);
                self.queue.jump_first_matching(self.column_formats.as_slice(), ctx);
            }
            InputResultEvent::Pop => {
                self.queue.recalculate_matched_items(self.column_formats.as_slice(), ctx);
            }
            InputResultEvent::Confirm => {}
            InputResultEvent::Cancel => {
                self.queue.set_filter_active(false);
                ctx.input.clear_buffer(self.queue.filter_buffer_id);
            }
            InputResultEvent::NoChange => {}
        }

        self.highlight_timeout(ctx);
        ctx.render()?;
        Ok(())
    }

    fn handle_insert_nav(&mut self, down: bool, handled: &mut bool, ctx: &mut Ctx) -> Result<()> {
        if self.plan_view(ctx) && self.plan.typing {
            *handled = true;
            return self.plan.insert_nav(down, ctx);
        }
        if self.find.as_ref().is_some_and(|f| f.typing) {
            if down {
                self.queue.next(ctx.config.scrolloff, false);
            } else {
                self.queue.prev(ctx.config.scrolloff, false);
            }
            *handled = true;
            ctx.render()?;
        }
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        // rormpc: `o` switches between physical Queue order and the plan view, keeping the selected and
        // marked songs by ID
        if event
            .actions
            .iter()
            .any(|a| matches!(a.as_queue(), Some(QueueActions::TogglePlanView)))
        {
            if event.claim_queue().is_none() {
                return Ok(());
            }
            if self.forced_plan.is_some() {
                status_info!("In Music the table follows w: the plan while weighted, else the queue order");
            } else {
                self.switch_view(ctx);
            }
            return Ok(ctx.render()?);
        }
        if self.plan_view(ctx) {
            if self.plan.action(event, ctx)? {
                return Ok(());
            }
            // Only file/ID-based actions reach the ordinary handler; its Dir stays in physical
            // queue order.
            self.map_plan_selection(ctx);
        }
        if self.find.is_some() && self.filtered_action(event, ctx)? {
            return Ok(());
        }
        if let Some(action) = event.claim_queue() {
            match action {
                QueueActions::Delete if !self.queue.marked().is_empty() => {
                    for range in self.queue.marked().ranges().rev() {
                        ctx.command(move |_, client| {
                            client.delete_from_queue(range.into())?;
                            Ok(())
                        });
                    }
                    self.queue.marked_mut().clear();
                    status_info!("Marked songs removed from queue");
                    ctx.render()?;
                }
                QueueActions::Delete => {
                    if let Some((idx, _)) = self.queue.selected_with_idx() {
                        ctx.command(move |_, client| {
                            client.delete_from_queue(SingleOrRange::single(idx))?;
                            Ok(())
                        });
                    } else {
                        status_error!("No song selected");
                    }
                }
                QueueActions::DeleteAll => {
                    modal!(
                        ctx,
                        ConfirmModal::builder()
                            .ctx(ctx)
                            .message(vec![
                                "Are you sure you want to clear the queue?",
                                "This action cannot be undone."
                            ])
                            .action(Action::Single {
                                on_confirm: Box::new(|ctx| {
                                    ctx.command(|_, client| Ok(client.clear()?));
                                    Ok(())
                                }),
                                confirm_label: Some("Clear"),
                                cancel_label: None,
                            })
                            .size((45, 6))
                            .build()
                    );
                }
                QueueActions::Play => {
                    if let Some(selected_song) = self.queue.selected() {
                        let id = selected_song.id;
                        ctx.command(move |_, client| {
                            client.play_id(id)?;
                            Ok(())
                        });
                    }
                }
                QueueActions::TogglePlanView => {} // handled before either Queue view claims input
                QueueActions::Find => self.start_find(ctx),
                QueueActions::FindVersions => self.find_versions(ctx),
                QueueActions::SaveSmartList | QueueActions::SmartLists => {
                    status_info!("Smart lists live in Music: S saves its filters, L opens the lists");
                }
                // rormpc: an exception to the Hits rules; the queue itself does not change
                QueueActions::PinSong | QueueActions::ExcludeSong => {
                    let kind = if matches!(action, QueueActions::PinSong) {
                        crate::ui::rormpc_exceptions::Kind::Pin
                    } else {
                        crate::ui::rormpc_exceptions::Kind::Exclude
                    };
                    if let Some(song) = self.queue.selected() {
                        crate::ui::rormpc_exceptions::open_for_queue_song(ctx, kind, song);
                    } else {
                        status_error!("No song selected");
                    }
                }
                QueueActions::JumpToCurrent => {
                    if let Some((idx, _)) = ctx.status.songid.and_then(|id| {
                        self.queue.items.iter().enumerate().find(|(_, song)| song.id == id)
                    }) {
                        let scrolloff =
                            if self.queue.selected_with_idx().is_some_and(|(i, _)| i == idx) {
                                usize::MAX
                            } else {
                                ctx.config.scrolloff
                            };
                        self.queue.select_idx(idx, scrolloff);
                        ctx.render()?;
                    } else {
                        status_info!("No song is currently playing");
                    }
                }

                QueueActions::SelectAlbum => {
                    if let Some(selected_idx) = self.queue.selected_idx() {
                        let mut album_ranges = self.queue.items.as_slice().to_album_ranges();

                        if let Some(range) = album_ranges.find(|r| r.contains(&selected_idx)) {
                            for idx in range {
                                self.queue.state.toggle_mark(idx);
                            }
                            ctx.render()?;
                        }
                    }
                }

                QueueActions::Shuffle if !self.queue.marked().is_empty() => {
                    for range in self.queue.marked().ranges().rev() {
                        ctx.command(move |_, client| {
                            client.shuffle(Some(range.into()))?;
                            Ok(())
                        });
                    }
                    status_info!("Shuffled selected songs");
                }
                QueueActions::Shuffle => {
                    ctx.command(move |_, client| {
                        client.shuffle(None)?;
                        Ok(())
                    });
                    status_info!("Shuffled the queue");
                }
                QueueActions::SortByColumn(_) if self.forced_plan == Some(true) => {
                    status_info!("The plan never sorts MPD; w (weighted off) shows the queue order");
                }
                QueueActions::SortByColumn(idx) => {
                    QueueHeaderPane::sort_by_column(self.column_formats.as_slice(), *idx, ctx)?;
                    ctx.render()?;
                }
                QueueActions::Sort { kind: Sort::Modal(opts) } => {
                    let modal = MenuModal::new(ctx)
                        .select_section(ctx, |mut sect| {
                            for opt in opts {
                                sect.add_item(opt.0.clone(), opt.0.clone());

                                let opts = opts.clone();
                                sect.action(move |ctx, value| {
                                    let Some((_, opts)) = opts.iter().find(|opt| opt.0 == value)
                                    else {
                                        // shouldn't happen since the options
                                        // are generated from
                                        // the same list, but just in case
                                        status_error!("Invalid option selected");
                                        return Ok(());
                                    };

                                    Self::sort(opts.clone(), ctx)
                                });
                            }

                            Some(sect)
                        })
                        .list_section(ctx, |section| Some(section.item("Cancel", |_ctx| Ok(()))))
                        .build();

                    modal!(ctx, modal);
                }
                QueueActions::Sort { kind: Sort::Tags(opts) } => {
                    Self::sort(opts.clone(), ctx)?;
                    ctx.render()?;
                }
                QueueActions::Unused => {}
                // rormpc: Play's own keys (its left column), nothing in a plain queue
                QueueActions::ToggleBrowse | QueueActions::PreviousGrouping | QueueActions::NextGrouping => {
                    event.abandon();
                }
            }
        } else if let Some(action) = event.claim_common().map(|v| v.to_owned()) {
            match action {
                CommonAction::Up => {
                    if !self.queue.is_empty() {
                        self.queue.prev(ctx.config.scrolloff, ctx.config.wrap_navigation);
                    }

                    ctx.render()?;
                }
                CommonAction::Down => {
                    if !self.queue.is_empty() {
                        self.queue.next(ctx.config.scrolloff, ctx.config.wrap_navigation);
                    }

                    ctx.render()?;
                }
                CommonAction::MoveUp if !self.queue.marked().is_empty() => {
                    if self.queue.is_empty() {
                        return Ok(());
                    }

                    if let Some(0) = self.queue.marked().first() {
                        return Ok(());
                    }

                    let ranges = self.queue.marked().ranges().collect_vec();
                    for range in ranges {
                        for idx in range.clone() {
                            let new_idx = idx.saturating_sub(1);
                            self.queue.items.swap(idx, new_idx);
                        }

                        let new_start_idx = range.start().saturating_sub(1);
                        ctx.app_event_sender
                            .send(AppEvent::IgnoreIdleEvent(IdleEvent::Playlist))?;
                        ctx.command(move |tx, client| {
                            let result = client.move_in_queue(
                                range.into(),
                                QueuePosition::Absolute(new_start_idx),
                            );
                            tx.send(AppEvent::UnIgnoreIdleEvent(IdleEvent::Playlist))?;
                            Ok(result?)
                        });
                    }

                    if let Some(start) = self.queue.marked().first() {
                        let new_idx = start.saturating_sub(1);
                        self.queue.select_idx(new_idx, ctx.config.scrolloff);
                    }

                    let mut new_marked =
                        self.queue.marked().iter().map(|i| i.saturating_sub(1)).collect();
                    std::mem::swap(self.queue.marked_mut(), &mut new_marked);

                    ctx.render()?;
                    return Ok(());
                }
                CommonAction::MoveDown if !self.queue.marked().is_empty() => {
                    if self.queue.is_empty() {
                        return Ok(());
                    }

                    if let Some(last_idx) = self.queue.marked().last()
                        && *last_idx == self.queue.len() - 1
                    {
                        return Ok(());
                    }

                    let ranges = self.queue.marked().ranges().rev().collect_vec();
                    for range in ranges {
                        for idx in range.clone().rev() {
                            let new_idx = idx.saturating_add(1);
                            self.queue.items.swap(idx, new_idx);
                        }

                        let new_start_idx = range.start().saturating_add(1);
                        ctx.app_event_sender
                            .send(AppEvent::IgnoreIdleEvent(IdleEvent::Playlist))?;
                        ctx.command(move |tx, client| {
                            let result = client.move_in_queue(
                                range.into(),
                                QueuePosition::Absolute(new_start_idx),
                            );
                            tx.send(AppEvent::UnIgnoreIdleEvent(IdleEvent::Playlist))?;
                            Ok(result?)
                        });
                    }

                    if let Some(start) = self.queue.marked().last() {
                        let new_idx = start.saturating_add(1);
                        self.queue.select_idx(new_idx, ctx.config.scrolloff);
                    }

                    let mut new_marked =
                        self.queue.marked().iter().map(|i| i.saturating_add(1)).collect();
                    std::mem::swap(self.queue.marked_mut(), &mut new_marked);

                    ctx.render()?;
                    return Ok(());
                }
                CommonAction::MoveUp => {
                    if self.queue.is_empty() {
                        return Ok(());
                    }

                    let Some(idx) = self.queue.selected_idx() else {
                        return Ok(());
                    };

                    if idx == 0 {
                        return Ok(());
                    }

                    let new_idx = idx.saturating_sub(1);
                    ctx.app_event_sender.send(AppEvent::IgnoreIdleEvent(IdleEvent::Playlist))?;
                    ctx.command(move |tx, client| {
                        let result = client.move_in_queue(
                            SingleOrRange::single(idx),
                            QueuePosition::Absolute(new_idx),
                        );
                        tx.send(AppEvent::UnIgnoreIdleEvent(IdleEvent::Playlist))?;
                        Ok(result?)
                    });
                    self.queue.select_idx(new_idx, ctx.config.scrolloff);
                    self.queue.items.swap(idx, new_idx);
                    ctx.render()?;
                }
                CommonAction::MoveDown => {
                    if self.queue.is_empty() {
                        return Ok(());
                    }

                    let Some(idx) = self.queue.selected_idx() else {
                        return Ok(());
                    };

                    let new_idx = (idx + 1).min(self.queue.len() - 1);
                    ctx.app_event_sender.send(AppEvent::IgnoreIdleEvent(IdleEvent::Playlist))?;
                    ctx.command(move |tx, client| {
                        let result = client.move_in_queue(
                            SingleOrRange::single(idx),
                            QueuePosition::Absolute(new_idx),
                        );
                        tx.send(AppEvent::UnIgnoreIdleEvent(IdleEvent::Playlist))?;
                        Ok(result?)
                    });
                    self.queue.select_idx(new_idx, ctx.config.scrolloff);
                    self.queue.items.swap(idx, new_idx);
                    ctx.render()?;
                }
                CommonAction::DownHalf => {
                    if !self.queue.is_empty() {
                        self.queue.next_half_viewport(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::UpHalf => {
                    if !self.queue.is_empty() {
                        self.queue.prev_half_viewport(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::PageDown => {
                    if !self.queue.is_empty() {
                        self.queue.next_viewport(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::PageUp => {
                    if !self.queue.is_empty() {
                        self.queue.prev_viewport(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::Bottom => {
                    if !self.queue.is_empty() {
                        self.queue.last();
                    }

                    ctx.render()?;
                }
                CommonAction::Top => {
                    if !self.queue.is_empty() {
                        self.queue.first();
                    }

                    ctx.render()?;
                }
                CommonAction::ScrollFocusedToTop => {
                    if !self.queue.is_empty() {
                        self.queue.scroll_focused_to_top(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::ScrollFocusedToMiddle => {
                    if !self.queue.is_empty() {
                        self.queue.scroll_focused_to_middle(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::ScrollFocusedToBottom => {
                    if !self.queue.is_empty() {
                        self.queue.scroll_focused_to_bottom(ctx.config.scrolloff);
                    }

                    ctx.render()?;
                }
                CommonAction::Right => {}
                CommonAction::Left => {}
                CommonAction::EnterSearch => {
                    ctx.input.insert_mode(self.queue.filter_buffer_id);
                    ctx.input.clear_buffer(self.queue.filter_buffer_id);
                    self.queue.set_filter_active(true);

                    ctx.render()?;
                }
                CommonAction::NextResult => {
                    self.queue.jump_next_matching(self.column_formats.as_slice(), ctx);

                    ctx.render()?;
                }
                CommonAction::PreviousResult => {
                    self.queue.jump_previous_matching(self.column_formats.as_slice(), ctx);

                    ctx.render()?;
                }
                CommonAction::Select => {
                    if self.queue.selected().is_some() {
                        self.queue.toggle_mark_selected();
                        self.queue.next(ctx.config.scrolloff, ctx.config.wrap_navigation);

                        ctx.render()?;
                    }
                }
                CommonAction::InvertSelection => {
                    self.queue.invert_marked();

                    ctx.render()?;
                }
                CommonAction::CopyToClipboard { kind: CopyContentsKind::Content(content) } => {
                    let items = self.items(content.all);
                    let (sep, format) = match &content.content {
                        CopyContent::DisplayedValue => (" ", &self.column_formats),
                        CopyContent::Metadata(props) => ("", props),
                    };

                    let content = items
                        .map(|(_, song)| <Song as DirStackItem>::format(song, format, sep, ctx))
                        .join("\n");

                    Clipboard::from(content).write_with_status();
                }
                CommonAction::CopyToClipboard { kind: CopyContentsKind::Modal(opts) } => {
                    let column_formats = self.column_formats.clone();

                    let items = self.items(false).map(|(_, b)| b.clone()).collect_vec();
                    let all_items = if opts.iter().any(|opt| opt.1.all) {
                        self.items(true).map(|(_, b)| b.clone()).collect_vec()
                    } else {
                        Vec::new()
                    };

                    let modal = create_copy_to_clipboard_modal(
                        &opts,
                        column_formats,
                        items,
                        all_items,
                        " ",
                        "",
                        ctx,
                    );

                    modal!(ctx, modal);
                }
                CommonAction::Close if !self.queue.marked().is_empty() => {
                    self.queue.marked_mut().clear();
                    ctx.render()?;
                }
                CommonAction::AddOptions { kind: AddKind::Action(options) } => {
                    let (enqueue, _hovered_song_idx) = self.enqueue_items(options.all);

                    if !enqueue.is_empty() {
                        Client::resolve_and_enqueue(
                            ctx,
                            enqueue,
                            options.position,
                            AutoplayKind::None,
                            None,
                            None,
                        );
                        self.queue.marked_mut().clear();
                    }
                }
                CommonAction::AddOptions { kind: AddKind::Modal(items) } => {
                    let opts = items
                        .into_iter()
                        .map(|(label, mut opts)| {
                            opts.autoplay = AutoplayKind::None;
                            let (enqueue, hovered_song_idx) = self.enqueue_items(opts.all);
                            (label, opts, (enqueue, hovered_song_idx))
                        })
                        .collect_vec();

                    modal!(ctx, create_add_modal(opts, ctx));
                    self.queue.marked_mut().clear();
                }
                CommonAction::ShowInfo => {
                    if let Some(selected_song) = self.queue.selected() {
                        modal!(
                            ctx,
                            InfoListModal::builder()
                                .items(SongCtx(selected_song, ctx))
                                .title("Song info")
                                .column_widths(&[30, 70])
                                .build()
                        );
                    } else {
                        status_error!("No song selected");
                    }
                }
                CommonAction::Delete => {}
                CommonAction::Rename => {}
                CommonAction::Close => {}
                CommonAction::FocusInput => {}
                // rormpc: t puts the marked songs (else the cursor row) into Up next; P is Browse's
                CommonAction::PlayNext => {
                    let files: Vec<String> = self.enqueue_items(false).0.into_iter().filter_map(|e| match e {
                        Enqueue::File { path } => Some(path),
                        _ => None,
                    }).collect();
                    crate::ui::rormpc_upnext::play_next(ctx, files);
                }
                CommonAction::PlayReplace => event.abandon(),
                CommonAction::Confirm => {} // queue has its own binding for
                // play
                CommonAction::PaneDown => {}
                CommonAction::PaneUp => {}
                CommonAction::PaneRight => {}
                CommonAction::PaneLeft => {}
                CommonAction::ContextMenu => {
                    self.open_context_menu(ctx);
                }
                CommonAction::Rate {
                    kind: RateKind::Value(value),
                    current: false,
                    min_rating: _,
                    max_rating: _,
                } => {
                    let items = self.enqueue_items(false).0;
                    ctx.command(move |_, client| {
                        client.set_sticker_multiple(RATING_STICKER, value.to_string(), items)?;
                        Ok(())
                    });
                }
                CommonAction::Rate {
                    kind: RateKind::ClearRating(),
                    current: false,
                    min_rating: _,
                    max_rating: _,
                } => {
                    let items = self.enqueue_items(false).0;
                    ctx.command(move |_, client| {
                        client.delete_sticker_multiple(RATING_STICKER, items)?;
                        Ok(())
                    });
                }
                CommonAction::Rate {
                    kind: RateKind::Modal { values, custom, like },
                    current: false,
                    min_rating,
                    max_rating,
                } => {
                    let items = self.enqueue_items(false).0;
                    modal!(
                        ctx,
                        create_rating_modal(
                            items,
                            values.as_slice(),
                            min_rating,
                            max_rating,
                            custom,
                            like,
                            ctx
                        )
                    );
                }
                CommonAction::Rate { kind: RateKind::Like(), current: false, .. } => {
                    let items = self.enqueue_items(false).0;
                    ctx.command(move |_, client| {
                        client.set_sticker_multiple(LIKE_STICKER, "2".to_string(), items)?;
                        Ok(())
                    });
                }
                CommonAction::Rate { kind: RateKind::Neutral(), current: false, .. } => {
                    let items = self.enqueue_items(false).0;
                    ctx.command(move |_, client| {
                        client.set_sticker_multiple(LIKE_STICKER, "1".to_string(), items)?;
                        Ok(())
                    });
                }
                CommonAction::Rate { kind: RateKind::Dislike(), current: false, .. } => {
                    let items = self.enqueue_items(false).0;
                    ctx.command(move |_, client| {
                        client.set_sticker_multiple(LIKE_STICKER, "0".to_string(), items)?;
                        Ok(())
                    });
                }
                CommonAction::Rate { kind: _, current: true, min_rating: _, max_rating: _ } => {
                    event.abandon();
                }
                CommonAction::Save { kind: _, current: true } => {
                    event.abandon();
                }
                CommonAction::Save {
                    kind: SaveKind::Playlist { name, all, duplicates_strategy },
                    current: false,
                } => {
                    let song_paths: Vec<String> =
                        self.items(all).map(|(_, song)| song.file.clone()).collect();
                    if song_paths.is_empty() {
                        status_warn!("No songs selected to save");
                        return Ok(());
                    }

                    add_to_playlist_or_show_modal(name, song_paths, duplicates_strategy, ctx);
                }
                CommonAction::Save {
                    kind: SaveKind::Modal { all, duplicates_strategy },
                    current: false,
                } => {
                    let song_paths: Vec<String> =
                        self.items(all).map(|(_, song)| song.file.clone()).collect();
                    if song_paths.is_empty() {
                        status_warn!("No songs selected to save");
                        return Ok(());
                    }
                    let modal = create_save_modal(song_paths, None, duplicates_strategy, ctx)?;
                    modal!(ctx, modal);
                }
                CommonAction::DeleteFromPlaylist {
                    kind: DeleteKind::Playlist { name, all, confirmation },
                } => {
                    let song_paths: HashSet<String> =
                        self.items(all).map(|(_, song)| song.file.clone()).collect();
                    if song_paths.is_empty() {
                        status_warn!("No songs selected to delete");
                        return Ok(());
                    }

                    delete_from_playlist_or_show_confirmation(
                        name,
                        &song_paths,
                        confirmation,
                        ctx,
                    )?;
                }
                CommonAction::DeleteFromPlaylist {
                    kind: DeleteKind::Modal { all, confirmation },
                } => {
                    let song_paths: HashSet<String> =
                        self.items(all).map(|(_, song)| song.file.clone()).collect();
                    if song_paths.is_empty() {
                        status_warn!("No songs selected to delete");
                        return Ok(());
                    }

                    let modal = create_delete_modal(song_paths, confirmation, ctx)?;
                    modal!(ctx, modal);
                }
            }
        } else if let Some(action) = event.claim_global() {
            match action {
                GlobalAction::ExternalCommand { command, prompt, .. } => {
                    let songs =
                        create_env(ctx, self.items(false).map(|(_, song)| song.file.as_str()));
                    if crate::ui::rormpc_actions::delete_menu_instead(ctx, command, &songs) {
                        // rormpc: Ctrl-x opens the delete menu
                    } else if *prompt {
                        let command = command.clone();
                        modal!(
                            ctx,
                            InputModal::new(ctx).title("Enter arguments").on_confirm(
                                move |_ctx, value| {
                                    let args = args::split_command_line(value)?;
                                    run_external(command, args, songs);
                                    Ok(())
                                },
                            )
                        );
                    } else {
                        run_external(command.clone(), Vec::new(), songs);
                    }
                }
                _ => {
                    event.abandon();
                }
            }
        }

        self.highlight_enabled = true;

        self.highlight_timeout(ctx);

        Ok(())
    }
}

impl QueuePane {
    fn scrollbar_area(&self) -> Option<Rect> {
        let area = self.areas[Areas::Scrollbar];
        if area.width > 0 { Some(area) } else { None }
    }
}

/// rormpc: rows of the queue matching the live filter (indices into `songs`): the exact matches in queue order,
/// else the close matches (typos). Matches artist, title, album and the file name, diacritic-folded.
pub(crate) fn find_matches(songs: &[Song], query: &str) -> crate::ui::rormpc_filter::Found {
    crate::ui::rormpc_filter::find(
        songs.iter().map(|song| {
            let tag = |k: &str| song.metadata.get(k).map(|v| v.last().to_owned()).unwrap_or_default();
            let file = song.file.rsplit('/').next().unwrap_or(&song.file);
            format!("{} {} {} {file}", tag("artist"), tag("title"), tag("album"))
        }),
        query,
    )
}

/// rormpc: the Queue's live filter. Typing narrows the queue to the matching songs in queue order; Enter plays
/// the selected one and shows the whole queue on it; Esc restores the cursor and the scroll. The filtered list
/// is a view rebuilt from MPD's queue by song id, so no action ever uses a filtered position as a queue position.
impl QueuePane {
    /// rormpc: "Find versions…" (`QueueActions::FindVersions`) for the song under the cursor.
    fn find_versions(&self, ctx: &Ctx) {
        if let Some(song) = self.queue.selected() {
            let view = (song.id, self.queue.state.offset());
            crate::ui::rormpc_versions::find_versions(ctx, &song.file, Some(view));
        } else {
            status_info!("No song selected");
        }
    }

    fn start_find(&mut self, ctx: &Ctx) {
        if let Some(f) = &mut self.find {
            f.typing = true;
            ctx.input.insert_mode(f.buffer);
            return;
        }
        let buffer = BufferId::new();
        ctx.input.create_buffer(buffer, None);
        ctx.input.insert_mode(buffer);
        self.find = Some(QueueFind {
            buffer,
            typing: true,
            query: String::new(),
            saved_id: self.queue.selected().map(|s| s.id),
            saved_offset: self.queue.state.offset(),
            close: false,
        });
        self.apply_find(ctx, false);
    }

    /// Rebuild the filtered rows from MPD's queue. `snap`: the cursor goes to the first match (a keystroke);
    /// otherwise it stays on its song while that is still shown.
    fn apply_find(&mut self, ctx: &Ctx, snap: bool) {
        let Some(query) = self.find.as_ref().map(|f| f.query.clone()) else { return };
        let keep = self.queue.selected().map(|s| s.id);
        let found = find_matches(&ctx.queue, &query);
        if let Some(f) = &mut self.find {
            f.close = found.close;
        }
        let rows: Vec<Song> = found.rows.into_iter().map(|i| ctx.queue[i].clone()).collect();
        let marked = crate::ui::rormpc_actions::remap_marks(&self.queue.items, self.queue.marked(), &rows);
        self.queue.items = rows;
        *self.queue.marked_mut() = marked;
        self.queue.state.set_content_and_viewport_len(self.queue.len(), self.areas[Areas::Table].height as usize);
        let idx = if snap { None } else { keep.and_then(|id| self.queue.items.iter().position(|s| s.id == id)) };
        self.queue.select_idx_opt((!self.queue.is_empty()).then(|| idx.unwrap_or(0)), ctx.config.scrolloff);
        self.recalculate_album_indices();
    }

    /// Leave the filter: `play` plays the selected song (by id) and shows it in the whole queue; otherwise the
    /// cursor and scroll from before the filter come back.
    fn end_find(&mut self, ctx: &Ctx, play: bool) {
        let Some(f) = self.find.take() else { return };
        ctx.input.destroy_buffer(f.buffer);
        let chosen = self.queue.selected().map(|s| s.id);
        let marked = crate::ui::rormpc_actions::remap_marks(&self.queue.items, self.queue.marked(), &ctx.queue);
        self.queue.items.clone_from(&ctx.queue);
        *self.queue.marked_mut() = marked;
        self.queue.state.set_content_and_viewport_len(self.queue.len(), self.areas[Areas::Table].height as usize);
        self.recalculate_album_indices();
        let position = |id: Option<u32>| id.and_then(|id| self.queue.items.iter().position(|s| s.id == id));
        if play {
            if let Some(id) = chosen {
                ctx.command(move |_, client| {
                    client.play_id(id)?;
                    Ok(())
                });
            }
            if let Some(idx) = position(chosen) {
                self.queue.select_idx(idx, usize::MAX); // centred
            }
        } else if let Some(idx) = position(f.saved_id) {
            self.queue.select_idx(idx, ctx.config.scrolloff);
            self.queue.state.set_offset(f.saved_offset);
        }
    }

    /// Actions while the queue is filtered. Moving through the rows, marks, info, ratings and saving the
    /// selected songs work as usual (they use song ids or files); deleting works on the selected or marked
    /// rows by id; anything that uses queue positions (moves, sorting, the context menu) waits until the filter
    /// is cleared. Returns whether the action was handled here.
    fn filtered_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<bool> {
        let close = crate::ui::rormpc_filter::binding(&ctx.config.keybinds.navigation, |a| {
            matches!(a, CommonAction::Close)
        })
        .unwrap_or_else(|| "Esc".to_owned());
        if let Some(action) = event.actions.iter().find_map(|a| a.as_queue()).cloned() {
            match action {
                QueueActions::Play => {
                    event.claim_queue();
                    self.end_find(ctx, true);
                }
                QueueActions::Find => {
                    event.claim_queue();
                    self.start_find(ctx);
                }
                QueueActions::Delete => {
                    event.claim_queue();
                    let ids: Vec<u32> = self.items(false).map(|(_, s)| s.id).collect();
                    if ids.is_empty() {
                        status_error!("No song selected");
                    } else {
                        let n = ids.len();
                        ctx.command(move |_, client| {
                            for id in ids {
                                client.delete_id(id)?;
                            }
                            Ok(())
                        });
                        self.queue.marked_mut().clear();
                        status_info!("{n} song(s) removed from the queue");
                    }
                }
                QueueActions::JumpToCurrent => {
                    self.end_find(ctx, false);
                    return Ok(false); // the usual jump, now on the whole queue
                }
                // by file: the filtered row is fine
                QueueActions::FindVersions | QueueActions::PinSong | QueueActions::ExcludeSong => return Ok(false),
                _ => {
                    event.claim_queue();
                    status_warn!("Not while the queue is filtered: {close} clears the filter");
                }
            }
            ctx.render()?;
            return Ok(true);
        }
        let Some(action) = event.actions.iter().find_map(|a| a.as_common()).cloned() else {
            return Ok(false);
        };
        match action {
            CommonAction::Up
            | CommonAction::Down
            | CommonAction::UpHalf
            | CommonAction::DownHalf
            | CommonAction::PageUp
            | CommonAction::PageDown
            | CommonAction::Top
            | CommonAction::Bottom
            | CommonAction::ScrollFocusedToTop
            | CommonAction::ScrollFocusedToMiddle
            | CommonAction::ScrollFocusedToBottom
            | CommonAction::Select
            | CommonAction::InvertSelection
            | CommonAction::ShowInfo
            | CommonAction::Rate { .. }
            | CommonAction::CopyToClipboard { .. }
            | CommonAction::Save { kind: SaveKind::Playlist { all: false, .. } | SaveKind::Modal { all: false, .. }, .. }
            | CommonAction::Save { current: true, .. }
            | CommonAction::DeleteFromPlaylist { .. }
            | CommonAction::PaneUp
            | CommonAction::PaneDown
            | CommonAction::PaneLeft
            | CommonAction::PaneRight => return Ok(false),
            CommonAction::Confirm => self.end_find(ctx, true),
            CommonAction::Close => self.end_find(ctx, false),
            CommonAction::EnterSearch | CommonAction::FocusInput => self.start_find(ctx),
            _ => status_warn!("Not while the queue is filtered: {close} clears the filter"),
        }
        event.claim_common();
        ctx.render()?;
        Ok(true)
    }

    fn render_find_line(&self, frame: &mut Frame, f: &QueueFind, ctx: &Ctx) {
        let area = self.areas[Areas::FilterArea];
        if area.height == 0 {
            return;
        }
        let count = if f.close {
            format!("0 exact · {} close", self.queue.len())
        } else {
            format!("{:>11}", format!("{}/{}", self.queue.len(), ctx.queue.len()))
        };
        let mut text = format!("FILTER / {}{}", f.query, if f.typing { "▏" } else { "" });
        if f.close {
            text.push_str(&format!("   Close matches ({})", self.queue.len()));
        }
        let [left, right] = Layout::horizontal([Constraint::Min(1), Constraint::Length(count.chars().count() as u16)])
            .areas(area);
        let style = if f.typing { ctx.config.theme.highlight_border_style } else { ctx.config.as_text_style() };
        frame.render_widget(Line::from(Span::styled(text, style)), left);
        frame.render_widget(Line::from(count).style(ctx.config.as_text_style()), right);
    }
}

#[derive(Default)]
struct QueueRow {
    cell_style: Option<Style>,
    cursor_style: Option<Style>,
    underlined: bool,
}

impl QueueRow {
    fn into_row<'a>(self, cells: impl Iterator<Item = Line<'a>>) -> Row<'a> {
        let mut row_style = Style::default();

        if let Some(style) = self.cell_style {
            row_style = row_style.patch(style);
        }

        if let Some(cursor) = self.cursor_style {
            row_style = row_style.patch(cursor);
        }

        if self.underlined {
            row_style = row_style.underlined();
        }

        let row = Row::new(
            cells
                .map(|mut line| {
                    if let Some(style) = self.cell_style {
                        line.style = line.style.patch(style);
                        for span in &mut line.spans {
                            span.style = span.style.patch(style);
                        }
                    }
                    if let Some(cursor) = self.cursor_style {
                        line.style = line.style.patch(cursor);
                        for span in &mut line.spans {
                            span.style = span.style.patch(cursor);
                        }
                    }
                    line
                })
                .collect::<Vec<_>>(),
        );

        row.style(row_style)
    }
}

#[cfg(test)]
mod rormpc_find_tests {
    use rmpc_mpd::commands::{Song, metadata_tag::MetadataTag};

    use super::find_matches;

    fn song(id: u32, artist: &str, title: &str, file: &str) -> Song {
        let metadata = [("artist", artist), ("title", title)]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), MetadataTag::Single(v.to_owned())))
            .collect();
        Song { id, file: file.to_owned(), metadata, ..Default::default() }
    }

    #[test]
    fn filter_keeps_queue_order_and_folds_diacritics() {
        let q = vec![
            song(7, "Kult", "Arahja", "a/1.mp3"),
            song(3, "Myslovitz", "Długość dźwięku samotności", "a/2.mp3"),
            song(9, "Lao Che", "Żółw", "a/3.mp3"),
            song(1, "Kult", "Polska", "b/Łódź nocą.mp3"),
        ];
        let rows = |query: &str| find_matches(&q, query).rows;
        assert_eq!(rows(""), vec![0, 1, 2, 3]);
        assert_eq!(rows("zolw"), vec![2]);
        assert_eq!(rows("dlugosc"), vec![1]);
        assert_eq!(rows("lodz"), vec![3]); // the file name counts
        assert_eq!(rows("kult"), vec![0, 3]); // queue order, not score order
        assert!(rows("qqq").is_empty());
        let typo = find_matches(&q, "myslowitz");
        assert_eq!((typo.rows, typo.close), (vec![1], true)); // close match: one typo
    }
}
