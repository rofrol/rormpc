//! rormpc: Hits pane. Shows the ranked chart hits written by the `hits` CLI (`hits ... --json PATH`), owned
//! songs as normal MPD songs (play/queue work as in any browser), missing ones as greyed placeholder rows.
//! The ranking stays in `hits`; this pane only reads its versioned JSON file.

use std::{collections::HashMap, path::PathBuf, time::SystemTime};

use anyhow::{Context, Result};
use enum_map::EnumMap;
use ratatui::{Frame, prelude::Rect, widgets::ListState};
use rmpc_mpd::{
    client::Client,
    commands::{Song, lsinfo::LsInfoEntry},
    mpd_client::MpdClient,
};
use serde::Deserialize;

use super::Pane;
use crate::{
    MpdQueryResult,
    config::{
        tabs::PaneType,
        theme::properties::{Property, SongProperty},
    },
    ctx::Ctx,
    shared::{keys::ActionEvent, macros::modal, mouse_event::MouseEvent},
    ui::{
        UiEvent,
        browser::BrowserPane,
        dir_or_song::DirOrSong,
        dirstack::DirStack,
        input::InputResultEvent,
        modals::info_list_modal::{InfoListModal, SongCtx},
        widgets::browser::{Browser, BrowserArea},
    },
};

const INIT: &str = "hits_init";

#[derive(Debug, Deserialize)]
struct HitsFile {
    version: u32,
    label: String,
    rows: Vec<HitsRow>,
}

#[derive(Debug, Clone, Deserialize)]
struct HitsRow {
    rank: u32,
    pct: f64,
    cohort: u32,
    artist: String,
    title: String,
    year: i32,
    #[serde(default)]
    genres: Vec<String>,
    #[serde(default)]
    file: Option<String>,
}

#[derive(Debug)]
pub struct HitsPane {
    path: PathBuf,
    rows: Vec<HitsRow>,
    label: String,
    loaded_mtime: Option<SystemTime>,
    stack: DirStack<DirOrSong, ListState>,
    browser: Browser<DirOrSong>,
    target_pane: PaneType,
}

impl HitsPane {
    pub fn new(path: String, target_pane: PaneType, format: Vec<Property<SongProperty>>, _ctx: &Ctx) -> Self {
        let browser = if format.is_empty() { Browser::new() } else { Browser::new().with_song_format(format) };
        Self {
            path: PathBuf::from(shellexpand_home(&path)),
            rows: Vec::new(),
            label: String::new(),
            loaded_mtime: None,
            stack: DirStack::default(),
            browser,
            target_pane,
        }
    }

    /// Re-read the JSON file when it changed and ask MPD for the owned songs.
    fn reload(&mut self, ctx: &Ctx) -> Result<()> {
        let mtime = std::fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        if mtime.is_none() || mtime == self.loaded_mtime {
            return Ok(());
        }
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("reading {}", self.path.display()))?;
        let file: HitsFile = serde_json::from_str(&text)
            .with_context(|| format!("parsing {}", self.path.display()))?;
        if file.version != 1 {
            anyhow::bail!("{}: unsupported hits file version {}", self.path.display(), file.version);
        }
        self.rows = file.rows;
        self.label = file.label;
        self.loaded_mtime = mtime;
        let files: Vec<String> = self.rows.iter().filter_map(|r| r.file.clone()).collect();
        ctx.query()
            .id(INIT)
            .replace_id(INIT)
            .target(self.target_pane.clone())
            .query(move |client| fetch_songs(client, &files));
        Ok(())
    }

    /// Rows in rank order: owned ones as songs tagged with their rank, missing ones as placeholders.
    fn build(&mut self, songs: Vec<Song>) {
        let mut by_file: HashMap<String, Song> = songs.into_iter().map(|s| (s.file.clone(), s)).collect();
        let items = self
            .rows
            .iter()
            .map(|r| {
                let rank = format!("#{} ({:.0}%)", r.rank, r.pct.ceil());
                match r.file.as_ref().and_then(|f| by_file.remove(f)) {
                    Some(mut song) => {
                        song.metadata.insert("hits_rank".to_owned(), rank.into());
                        song.metadata.insert("hits_year".to_owned(), r.year.to_string().into());
                        song.metadata.insert("hits_genres".to_owned(), r.genres.join(", ").into());
                        DirOrSong::Song(song)
                    }
                    None => {
                        let name = format!("✗ {rank}  {} - {}  ({})", r.artist, r.title, r.year);
                        DirOrSong::Dir {
                            name: name.clone(),
                            display_name: Some(name),
                            full_path: String::new(),
                            last_modified: chrono::DateTime::<chrono::Utc>::default(),
                            playlist: false,
                            metadata: HashMap::from([
                                ("cohort".to_owned(), r.cohort.to_string()),
                                ("genres".to_owned(), r.genres.join(", ")),
                            ]),
                        }
                    }
                }
            })
            .collect();
        self.stack = DirStack::new(items);
    }
}

fn shellexpand_home(path: &str) -> String {
    match (path.strip_prefix("~/"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}/{rest}"),
        _ => path.to_owned(),
    }
}

/// One lsinfo per file: a stale path (file deleted since `hits` ran) is skipped instead of failing the batch.
fn fetch_songs(client: &mut Client<'_>, files: &[String]) -> Result<MpdQueryResult> {
    let mut songs = Vec::with_capacity(files.len());
    for file in files {
        if let Ok(info) = client.lsinfo(Some(file)) {
            songs.extend(info.0.into_iter().filter_map(|e| match e {
                LsInfoEntry::File(song) => Some(song),
                _ => None,
            }));
        }
    }
    Ok(MpdQueryResult::SongsList { data: songs, path: None })
}

impl Pane for HitsPane {
    fn render(&mut self, frame: &mut Frame, area: Rect, ctx: &Ctx) -> Result<()> {
        self.browser.render(area, frame.buffer_mut(), &mut self.stack, ctx);
        Ok(())
    }

    fn before_show(&mut self, ctx: &Ctx) -> Result<()> {
        self.reload(ctx)
    }

    fn on_event(&mut self, event: &mut UiEvent, is_visible: bool, ctx: &Ctx) -> Result<()> {
        match event {
            UiEvent::Database | UiEvent::Reconnected => {
                self.loaded_mtime = None; // library changed: owned songs may have changed too
                if is_visible {
                    self.reload(ctx)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_action(&mut self, event: &mut ActionEvent, ctx: &mut Ctx) -> Result<()> {
        self.handle_common_action(event, ctx)?;
        self.handle_global_action(event, ctx)?;
        Ok(())
    }

    fn handle_insert_mode(&mut self, kind: InputResultEvent, ctx: &mut Ctx) -> Result<()> {
        BrowserPane::handle_insert_mode(self, kind, ctx)?;
        Ok(())
    }

    fn handle_mouse_event(&mut self, event: MouseEvent, ctx: &Ctx) -> Result<()> {
        self.handle_mouse_action(event, ctx)
    }

    fn on_query_finished(
        &mut self,
        id: &'static str,
        data: MpdQueryResult,
        _is_visible: bool,
        ctx: &Ctx,
    ) -> Result<()> {
        if id != INIT {
            return Ok(());
        }
        let MpdQueryResult::SongsList { data: songs, .. } = data else {
            return Ok(());
        };
        self.build(songs);
        ctx.render()?;
        Ok(())
    }
}

impl BrowserPane<DirOrSong> for HitsPane {
    fn stack(&self) -> &DirStack<DirOrSong, ListState> {
        &self.stack
    }

    fn stack_mut(&mut self) -> &mut DirStack<DirOrSong, ListState> {
        &mut self.stack
    }

    fn browser_areas(&self) -> EnumMap<BrowserArea, Rect> {
        self.browser.areas
    }

    fn list_songs_in_item(
        &self,
        item: DirOrSong,
    ) -> impl FnOnce(&mut Client<'_>) -> Result<Vec<Song>> + Send + Sync + Clone + 'static {
        // missing songs (placeholder rows) have nothing to play or queue
        move |_| {
            Ok(match item {
                DirOrSong::Song(song) => vec![song],
                DirOrSong::Dir { .. } => vec![],
            })
        }
    }

    fn fetch_data(&self, _selected: &DirOrSong, _ctx: &Ctx) -> Result<()> {
        Ok(())
    }

    fn show_info(&self, item: &DirOrSong, ctx: &Ctx) -> Result<()> {
        let DirOrSong::Song(song) = item else {
            return Ok(());
        };
        modal!(
            ctx,
            InfoListModal::builder()
                .items(SongCtx(song, ctx))
                .title("Hits song info")
                .column_widths(&[30, 70])
                .build()
        );
        Ok(())
    }
}
