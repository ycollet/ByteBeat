use std::{path::PathBuf, sync::mpsc::TryRecvError, time::Duration};

use eframe::egui::{self, Color32, RichText, Vec2};
use serde::{Deserialize, Serialize};

use crate::{
    audio::{Audio, Command},
    catalog,
    engine::Mode,
    songs::{self, Song},
    upstream::{self, Download, Event},
    visualizer::{self, Graph, Visualizer},
};

const GREEN: Color32 = Color32::from_rgb(100, 224, 150);
const BLUE: Color32 = Color32::from_rgb(99, 184, 241);
const STORAGE_KEY: &str = "bytebeat-project-v1";
const LARGE_FILE_BYTES: usize = 256 * 1024;
const PREVIEW_BYTES: usize = 4096;
const EDITOR_ID: &str = "bytebeat-source-editor";

fn code_preview(code: &str) -> &str {
    let mut end = code.len().min(PREVIEW_BYTES);
    while !code.is_char_boundary(end) {
        end -= 1;
    }
    &code[..end]
}

#[derive(Default)]
struct CodeEditor {
    full_edit: bool,
    acknowledged: bool,
    large_paste_blocked: bool,
}

impl CodeEditor {
    fn guarded(&self, code: &str) -> bool {
        code.len() > LARGE_FILE_BYTES && !self.full_edit
    }

    fn show(&mut self, ui: &mut egui::Ui, code: &mut String) {
        if !self.full_edit && ui.memory(|memory| memory.has_focus(egui::Id::new(EDITOR_ID))) {
            ui.input_mut(|input| {
                input.events.retain(|event| {
                    if matches!(event, egui::Event::Paste(text) | egui::Event::Text(text) if text.len() > LARGE_FILE_BYTES)
                    {
                        self.large_paste_blocked = true;
                        false
                    } else {
                        true
                    }
                });
            });
        }
        if code.len() > LARGE_FILE_BYTES || self.large_paste_blocked {
            ui.colored_label(
                Color32::YELLOW,
                "Large script: full editing can use gigabytes of memory and freeze the interface.",
            );
            if self.full_edit {
                if ui.button("Return to safe preview").clicked() {
                    *self = Self::default();
                }
            } else {
                if self.large_paste_blocked {
                    ui.label("Large paste was not applied. Enable full editing, then paste again.");
                }
                if self.guarded(code) {
                    ui.label("Read-only preview of the first 4 KiB. Playback and saving use the complete source.");
                }
                ui.checkbox(
                    &mut self.acknowledged,
                    "I understand the memory and responsiveness risks.",
                );
                if ui
                    .add_enabled(self.acknowledged, egui::Button::new("Open full editor"))
                    .clicked()
                {
                    self.full_edit = true;
                }
            }
        }
        let height = (ui.available_height() - 4.0).max(100.0);
        egui::ScrollArea::both().max_height(height).show(ui, |ui| {
            if self.guarded(code) {
                let mut preview = code_preview(code);
                ui.add_sized(
                    [ui.available_width(), height],
                    egui::TextEdit::multiline(&mut preview)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY),
                );
            } else {
                ui.add_sized(
                    [ui.available_width(), height],
                    egui::TextEdit::multiline(code)
                        .id(egui::Id::new(EDITOR_ID))
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .lock_focus(true),
                );
            }
        });
    }
}

#[derive(Default)]
struct LibrarySearch {
    key: Option<(String, bool)>,
    matches: Vec<usize>,
}

impl LibrarySearch {
    fn invalidate(&mut self) {
        self.key = None;
    }

    fn update(&mut self, search: &str, only_saved: bool, demos: &[Song], saved: &[Song]) -> bool {
        let query = search.to_lowercase();
        if self
            .key
            .as_ref()
            .is_some_and(|(cached, filter)| cached == &query && *filter == only_saved)
        {
            return false;
        }
        self.matches.clear();
        self.matches.extend(
            demos
                .iter()
                .chain(saved.iter())
                .enumerate()
                .filter(|(i, song)| {
                    (!only_saved || *i >= demos.len())
                        && (query.is_empty()
                            || format!("{} {} {}", song.title, song.author, song.code)
                                .to_lowercase()
                                .contains(&query))
                })
                .map(|(i, _)| i),
        );
        self.key = Some((query, only_saved));
        true
    }
}

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
enum Scope {
    Waveform,
    Spectrum,
    Combined,
    #[default]
    Unspecified,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Project {
    code: String,
    title: String,
    mode: Mode,
    rate: f64,
    volume: f32,
    dc_blocker: bool,
    #[serde(skip_serializing)]
    scope: Scope,
    graphs: Vec<Graph>,
    saved: Vec<Song>,
}

impl Default for Project {
    fn default() -> Self {
        let song = songs::demos().remove(0);
        Self {
            code: song.code,
            title: song.title,
            mode: song.mode,
            rate: song.rate,
            volume: 0.25,
            dc_blocker: true,
            scope: Scope::Unspecified,
            graphs: visualizer::default_graphs(),
            saved: vec![],
        }
    }
}

impl Project {
    fn migrate_scope(&mut self) {
        self.graphs = match std::mem::take(&mut self.scope) {
            Scope::Waveform => visualizer::waveform_graphs(),
            Scope::Spectrum => visualizer::spectrum_graphs(),
            Scope::Combined => visualizer::default_graphs(),
            Scope::Unspecified => return,
        };
    }
}

pub struct Composer {
    project: Project,
    audio: Audio,
    demos: Vec<Song>,
    search: String,
    only_saved: bool,
    library_search: LibrarySearch,
    editor: CodeEditor,
    loaded: Option<(String, Mode, f64)>,
    visualizer: Visualizer,
    catalog_download: Option<Download>,
    download_root: Option<PathBuf>,
    catalog_status: Option<String>,
    catalog_error: Option<String>,
}

impl Composer {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        initial: Option<Song>,
        catalog: Vec<Song>,
        memory_limit_mb: u32,
        use_jack: bool,
        download_to: Option<PathBuf>,
    ) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        cc.egui_ctx.style_mut(|style| {
            style.visuals.panel_fill = Color32::from_rgb(24, 27, 31);
            style.visuals.window_fill = Color32::from_rgb(24, 27, 31);
            style.visuals.selection.bg_fill = Color32::from_rgb(37, 87, 61);
            style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        });
        let mut project: Project = cc
            .storage
            .and_then(|storage| eframe::get_value(storage, STORAGE_KEY))
            .unwrap_or_default();
        project.migrate_scope();
        project.volume = if project.volume.is_finite() {
            project.volume.clamp(0.0, 1.0)
        } else {
            0.25
        };
        project.rate = if project.rate.is_finite() {
            project.rate.clamp(1_000.0, 192_000.0)
        } else {
            8_000.0
        };
        if let Some(song) = initial {
            project.code = song.code;
            project.title = song.title;
            project.mode = song.mode;
            project.rate = song.rate;
        }
        let audio = Audio::new(memory_limit_mb, use_jack);
        audio.volume(project.volume);
        audio.dc_blocker(project.dc_blocker);
        let catalog_download = download_to
            .as_ref()
            .map(|root| Download::start(root.clone()));
        let catalog_status = download_to
            .as_ref()
            .map(|_| "Downloading upstream songs...".into());
        Self {
            project,
            audio,
            demos: songs::demos().into_iter().chain(catalog).collect(),
            search: String::new(),
            only_saved: false,
            library_search: LibrarySearch::default(),
            editor: CodeEditor::default(),
            loaded: None,
            visualizer: Visualizer::new(cc.gl.as_ref()),
            catalog_download,
            download_root: download_to,
            catalog_status,
            catalog_error: None,
        }
    }

    fn poll_catalog(&mut self) {
        while let Some(download) = self.catalog_download.as_ref() {
            let event = download.receiver.try_recv();
            let result = match event {
                Ok(Event::Progress(message)) => {
                    log::info!("{message}");
                    self.catalog_status = Some(message);
                    continue;
                }
                Ok(Event::Finished(result)) => result,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    Err("Catalog download worker disconnected unexpectedly.".into())
                }
            };
            self.catalog_download = None;
            match result.and_then(|path| catalog::load_songs(&path)) {
                Ok(songs) => {
                    self.catalog_status = Some(format!("Loaded {} upstream songs", songs.len()));
                    self.demos.extend(songs);
                    self.library_search.invalidate();
                    self.download_root = None;
                }
                Err(error) => {
                    log::error!("Upstream catalog download failed: {error}");
                    self.catalog_status = None;
                    self.catalog_error = Some(error);
                }
            }
            break;
        }
    }

    fn load(&mut self, play: bool) {
        self.audio.send(Command::Load {
            code: self.project.code.clone(),
            mode: self.project.mode,
            rate: self.project.rate,
            play,
        });
        self.loaded = Some((
            self.project.code.clone(),
            self.project.mode,
            self.project.rate,
        ));
    }

    fn dirty(&self) -> bool {
        self.loaded.as_ref().is_none_or(|(code, mode, rate)| {
            code != &self.project.code || *mode != self.project.mode || *rate != self.project.rate
        })
    }

    fn toggle(&mut self, playing: bool) {
        if playing {
            self.audio.send(Command::Play(false));
        } else if self.dirty() {
            self.load(true);
        } else {
            self.audio.send(Command::Play(true));
        }
    }

    fn save_song(&mut self) {
        let song = Song {
            title: self.project.title.clone(),
            author: "My songs".into(),
            code: self.project.code.clone(),
            rate: self.project.rate,
            mode: self.project.mode,
        };
        if let Some(existing) = self
            .project
            .saved
            .iter_mut()
            .find(|s| s.title == song.title)
        {
            *existing = song;
        } else {
            self.project.saved.push(song);
        }
        self.library_search.invalidate();
    }

    fn library(&mut self, ui: &mut egui::Ui) {
        ui.heading("Song library");
        if let Some(message) = &self.catalog_status {
            ui.label(message);
        }
        if let Some(error) = &self.catalog_error {
            ui.colored_label(
                Color32::from_rgb(255, 130, 120),
                format!("Upstream download failed: {error}"),
            );
            if let Some(root) = &self.download_root
                && ui.button("Retry download").clicked()
            {
                self.catalog_download = Some(upstream::Download::start(root.clone()));
                self.catalog_error = None;
                self.catalog_status = Some("Retrying upstream download...".into());
            }
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search title, author, code...")
                .desired_width(f32::INFINITY),
        );
        ui.checkbox(&mut self.only_saved, "My songs only");
        ui.separator();
        let mut selected = None;
        let mut remove = None;
        self.library_search.update(
            &self.search,
            self.only_saved,
            &self.demos,
            &self.project.saved,
        );
        let filtered = &self.library_search.matches;
        ui.label(format!("{} songs", filtered.len()));
        egui::ScrollArea::vertical().show_rows(ui, 132.0, filtered.len(), |ui, rows| {
            for row in rows {
                let i = filtered[row];
                let song = if i < self.demos.len() {
                    &self.demos[i]
                } else {
                    &self.project.saved[i - self.demos.len()]
                };
                let saved = i >= self.demos.len();
                ui.push_id(i, |ui| {
                    ui.allocate_ui_with_layout(
                        Vec2::new(ui.available_width(), 132.0),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.group(|ui| {
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(
                                            egui::Button::selectable(
                                                self.project.title == song.title,
                                                RichText::new(&song.title).color(GREEN).strong(),
                                            )
                                            .truncate(),
                                        )
                                        .clicked()
                                    {
                                        selected = Some(song.clone());
                                    }
                                    if saved
                                        && ui
                                            .small_button("x")
                                            .on_hover_text("Delete saved song")
                                            .clicked()
                                    {
                                        remove = Some(i - self.demos.len());
                                    }
                                });
                                ui.add(
                                    egui::Label::new(RichText::new(&song.author).small().weak())
                                        .truncate(),
                                );
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!(
                                            "{}  |  {} Hz  |  {} bytes",
                                            song.mode.label(),
                                            song.rate,
                                            song.code.len()
                                        ))
                                        .small()
                                        .color(BLUE),
                                    )
                                    .truncate(),
                                );
                                let preview: String = song.code.chars().take(100).collect();
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(preview).monospace().small().weak(),
                                    )
                                    .truncate(),
                                );
                                if ui.small_button("Load & play").clicked() {
                                    selected = Some(song.clone());
                                }
                            });
                        },
                    );
                });
            }
        });
        if let Some(index) = remove {
            self.project.saved.remove(index);
            self.library_search.invalidate();
        }
        if let Some(song) = selected {
            self.project.code = song.code;
            self.project.title = song.title;
            self.project.mode = song.mode;
            self.project.rate = song.rate;
            self.editor = CodeEditor::default();
            self.load(true);
        }
    }
}

impl eframe::App for Composer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_catalog();
        let (playing, position, frames, rate, error, generation) = {
            let state = self
                .audio
                .snapshot
                .lock()
                .expect("audio snapshot mutex poisoned");
            (
                state.playing,
                state.position,
                state.frames.clone(),
                state.output_rate,
                state.error.clone(),
                state.generation,
            )
        };
        self.visualizer.set_generation(generation);
        if ctx.input_mut(|i| {
            i.consume_key(egui::Modifiers::CTRL, egui::Key::Enter)
                || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter)
        }) {
            self.load(true);
        }

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(RichText::new("BYTEBEAT").color(GREEN));
                ui.label(RichText::new("COMPOSER").weak());
                ui.separator();
                ui.label("Native algorithmic music studio");
            });
        });
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            if let Some(error) = error {
                ui.colored_label(Color32::from_rgb(255, 130, 120), error);
            } else {
                ui.horizontal(|ui| {
                    ui.label(if playing { "Playing" } else { "Paused" });
                    ui.separator();
                    ui.label(format!(
                        "Output: {rate} Hz  |  Underrun frames: {}",
                        self.audio.underruns()
                    ));
                    ui.label(RichText::new("Ctrl/Cmd+Enter: compile & restart").weak());
                });
            }
        });
        egui::SidePanel::right("library")
            .default_width(310.0)
            .min_width(230.0)
            .resizable(true)
            .show(ctx, |ui| self.library(ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            let seconds = position as f64 / self.loaded.as_ref().map_or(self.project.rate, |(_, _, rate)| *rate);
            if self.visualizer.show(ui, &mut self.project.graphs, &frames, rate, seconds) {
                self.toggle(playing);
            }
            ui.horizontal_wrapped(|ui| {
                if ui.button(if playing { "Pause" } else { "Play" }).clicked() {
                    self.toggle(playing);
                }
                if ui.button("Stop").clicked() { self.load(false); }
                if ui.button("Restart").clicked() { self.load(playing); }
                ui.separator();
                ui.label(RichText::new(format!("t = {position}  /  {:.2} s",
                    position as f64 / self.loaded.as_ref().map_or(self.project.rate, |(_, _, rate)| *rate))).monospace());
                if ui.add(egui::Slider::new(&mut self.project.volume, 0.0..=1.0).text("Volume")).changed() {
                    self.audio.volume(self.project.volume);
                }
                if ui.checkbox(&mut self.project.dc_blocker, "DC blocker")
                    .on_hover_text("Remove DC offset with a 10 Hz high-pass filter per output channel. Not a substitute for safe listening volume.")
                    .changed() {
                    self.audio.dc_blocker(self.project.dc_blocker);
                }
            });
            ui.horizontal_wrapped(|ui| {
                let previous = (self.project.mode, self.project.rate);
                egui::ComboBox::from_id_salt("mode").selected_text(self.project.mode.label())
                    .show_ui(ui, |ui| {
                        for mode in Mode::ALL {
                            ui.selectable_value(&mut self.project.mode, mode, mode.label());
                        }
                    });
                ui.label("Sample rate");
                ui.add(egui::DragValue::new(&mut self.project.rate).range(1_000..=192_000).suffix(" Hz"));
                egui::ComboBox::from_id_salt("rates").selected_text("Presets").show_ui(ui, |ui| {
                    for rate in [4_000, 8_000, 11_025, 16_000, 22_050, 32_000, 44_100, 48_000] {
                        ui.selectable_value(&mut self.project.rate, f64::from(rate), rate.to_string());
                    }
                });
                if previous != (self.project.mode, self.project.rate) && playing {
                    self.load(true);
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Title");
                ui.add(egui::TextEdit::singleline(&mut self.project.title).desired_width(220.0));
                if ui.button("Save to my songs").on_hover_text("An existing song with this title is replaced").clicked() {
                    self.save_song();
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("JAVASCRIPT EDITOR").strong().color(GREEN));
                ui.label(format!("{} bytes", self.project.code.len()));
                if ui.button("Apply & play").clicked() { self.load(true); }
                if self.dirty() { ui.label(RichText::new("Unapplied changes").color(Color32::YELLOW)); }
            });
            ui.label(RichText::new("t = sample index  |  Math aliases: sin, cos, PI, int...  |  Stereo: [left, right]")
                .small().weak());
            self.editor.show(ui, &mut self.project.code);
        });
        if playing {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            // Receive compile errors and device initialization results while idle.
            ctx.request_repaint_after(Duration::from_millis(150));
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, &self.project);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.visualizer.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_scope_settings_migrate_using_native_storage_format() {
        for (scope, expected) in [
            ("Waveform", visualizer::waveform_graphs()),
            ("Spectrum", visualizer::spectrum_graphs()),
            ("Combined", visualizer::default_graphs()),
        ] {
            let mut storage = MemoryStorage(format!("(scope:{scope})"));
            let mut project: Project = eframe::get_value(&storage, STORAGE_KEY).unwrap();
            project.migrate_scope();
            assert_eq!(project.graphs, expected);
            project.migrate_scope();
            assert_eq!(project.graphs, expected);
            eframe::set_value(&mut storage, STORAGE_KEY, &project);
            assert!(!storage.0.contains("scope:"));
            let mut restored: Project = eframe::get_value(&storage, STORAGE_KEY).unwrap();
            restored.migrate_scope();
            assert_eq!(restored.graphs, expected);
        }
    }

    #[test]
    fn graph_settings_and_empty_list_persist_without_restoring_defaults() {
        let mut storage = MemoryStorage::default();
        let mut project: Project = serde_json::from_value(serde_json::json!({
            "graphs": [{
                "kind": "Bits",
                "channel": "Right",
                "color": [12, 34, 56],
                "opacity": 87
            }, {
                "kind": {"Shader": 34},
                "channel": "Mix",
                "color": [255, 128, 64],
                "opacity": 128
            }]
        }))
        .unwrap();
        for _ in 0..2 {
            eframe::set_value(&mut storage, STORAGE_KEY, &project);
            let mut restored: Project = eframe::get_value(&storage, STORAGE_KEY).unwrap();
            restored.migrate_scope();
            assert_eq!(restored.graphs, project.graphs);
            project.graphs.clear();
        }
    }

    #[test]
    fn dc_blocker_defaults_on_and_persists_when_disabled() {
        let mut project: Project = serde_json::from_str("{}").unwrap();
        assert!(project.dc_blocker);
        project.dc_blocker = false;
        let json = serde_json::to_string(&project).unwrap();
        let restored: Project = serde_json::from_str(&json).unwrap();
        assert!(!restored.dc_blocker);
    }

    fn editor_frame(
        ctx: &egui::Context,
        editor: &mut CodeEditor,
        code: &mut String,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    editor.show(ui, code);
                });
            },
        )
    }

    fn click_editor_label(
        ctx: &egui::Context,
        editor: &mut CodeEditor,
        code: &mut String,
        output: &egui::FullOutput,
        label: &str,
    ) -> egui::FullOutput {
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + egui::vec2(2.0, 2.0))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing editor control: {label}"));
        for pressed in [true, false] {
            let result = editor_frame(
                ctx,
                editor,
                code,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            if !pressed {
                return result;
            }
        }
        unreachable!()
    }

    #[test]
    fn full_editing_requires_acknowledgment_and_can_return_to_safe_preview() {
        let ctx = egui::Context::default();
        let mut editor = CodeEditor::default();
        let original = "a".repeat(LARGE_FILE_BYTES + 1);
        let mut code = original.clone();
        let output = editor_frame(&ctx, &mut editor, &mut code, vec![]);
        let output = click_editor_label(&ctx, &mut editor, &mut code, &output, "Open full editor");
        assert!(editor.guarded(&code));
        let output = click_editor_label(
            &ctx,
            &mut editor,
            &mut code,
            &output,
            "I understand the memory and responsiveness risks.",
        );
        assert!(editor.acknowledged);
        assert!(editor.guarded(&code));
        click_editor_label(&ctx, &mut editor, &mut code, &output, "Open full editor");
        assert!(!editor.guarded(&code));
        let output = editor_frame(&ctx, &mut editor, &mut code, vec![]);
        click_editor_label(
            &ctx,
            &mut editor,
            &mut code,
            &output,
            "Return to safe preview",
        );
        assert!(editor.guarded(&code));
        assert!(!editor.acknowledged);
        assert_eq!(code, original);
    }

    #[test]
    fn oversized_pastes_cannot_bypass_the_initial_editor_guard() {
        let ctx = egui::Context::default();
        let mut editor = CodeEditor::default();
        let mut code = "128".to_owned();
        editor_frame(&ctx, &mut editor, &mut code, vec![]);
        ctx.memory_mut(|memory| memory.request_focus(egui::Id::new(EDITOR_ID)));
        editor_frame(
            &ctx,
            &mut editor,
            &mut code,
            vec![egui::Event::Paste("a".repeat(7_000_000))],
        );
        assert_eq!(code, "128");
        assert!(editor.large_paste_blocked);
        assert!(!editor.full_edit);
    }

    #[test]
    fn large_file_threshold_and_utf8_preview_are_bounded() {
        let editor = CodeEditor::default();
        assert!(!editor.guarded(&"a".repeat(LARGE_FILE_BYTES)));
        assert!(editor.guarded(&"a".repeat(LARGE_FILE_BYTES + 1)));
        let code = format!(
            "{}{}",
            "a".repeat(PREVIEW_BYTES - 1),
            "\u{1f3b5}".repeat(100_000)
        );
        let preview = code_preview(&code);
        assert_eq!(preview.len(), PREVIEW_BYTES - 1);
        assert!(preview.len() <= PREVIEW_BYTES);
        assert!(code.starts_with(preview));
        assert_eq!(code_preview("short source"), "short source");
    }

    #[test]
    fn large_file_preview_never_lays_out_or_modifies_the_complete_source() {
        let ctx = egui::Context::default();
        let mut editor = CodeEditor::default();
        let original = "// complete source\n".repeat(400_000);
        let mut code = original.clone();
        for _ in 0..3 {
            let output = editor_frame(&ctx, &mut editor, &mut code, vec![]);
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(texts.contains(&code_preview(&original)));
            assert!(texts.iter().all(|text| text.len() <= PREVIEW_BYTES));
            assert_eq!(code, original);
            assert!(editor.guarded(&code));
        }
    }

    #[test]
    fn library_search_preserves_matching_semantics_and_reuses_unchanged_queries() {
        let mut demos = songs::demos();
        demos[0].title = "CAF\u{c9}".into();
        demos[0].author = "Tester".into();
        demos[0].code = "needle".into();
        let saved = vec![demos[0].clone()];
        let mut search = LibrarySearch::default();
        assert!(search.update("", false, &demos, &saved));
        assert_eq!(search.matches, (0..=demos.len()).collect::<Vec<_>>());
        assert!(search.update("caf\u{e9} tester needle", false, &demos, &saved));
        assert_eq!(search.matches, [0, demos.len()]);
        assert!(!search.update("CAF\u{c9} TESTER NEEDLE", false, &demos, &saved));
        assert!(search.update("needle", true, &demos, &saved));
        assert_eq!(search.matches, [demos.len()]);
        assert!(search.update("not present", false, &demos, &saved));
        assert!(search.matches.is_empty());
        assert!(!search.update("not present", false, &demos, &saved));
    }

    #[test]
    fn invalidation_handles_downloads_saves_replacements_and_deletions() {
        let mut demos = songs::demos();
        let mut saved = Vec::new();
        let mut search = LibrarySearch::default();
        search.update("marker", false, &demos, &saved);
        let mut song = demos[0].clone();
        song.title = "marker".into();
        demos.push(song.clone());
        search.invalidate();
        assert!(search.update("marker", false, &demos, &saved));
        assert_eq!(search.matches, [demos.len() - 1]);
        saved.push(song);
        search.invalidate();
        search.update("marker", true, &demos, &saved);
        assert_eq!(search.matches, [demos.len()]);
        saved[0].title = "replacement".into();
        search.invalidate();
        search.update("marker", true, &demos, &saved);
        assert!(search.matches.is_empty());
        saved.remove(0);
        search.invalidate();
        search.update("", true, &demos, &saved);
        assert!(search.matches.is_empty());
    }

    #[test]
    #[ignore = "manual release-mode measurement using the bundled upstream catalog"]
    fn benchmark_cached_catalog_search() {
        use std::{hint::black_box, path::Path, time::Instant};
        let library = catalog::load_songs(Path::new("data/catalog.json")).unwrap();
        let mut search = LibrarySearch::default();
        let start = Instant::now();
        assert!(search.update("no-such-song-9ce145", false, &library, &[]));
        let first = start.elapsed();
        let start = Instant::now();
        for _ in 0..100_000 {
            assert!(!search.update(black_box("no-such-song-9ce145"), false, &library, &[]));
            black_box(&search.matches);
        }
        println!(
            "{} songs: first search {:.3} ms; cached search {:.3} us/update",
            library.len(),
            first.as_secs_f64() * 1000.0,
            start.elapsed().as_secs_f64() * 1e6 / 100_000.0,
        );
    }

    #[test]
    #[ignore = "manual release-mode measurement using the largest bundled upstream script"]
    fn benchmark_large_editor_preview() {
        use std::{hint::black_box, path::Path, time::Instant};
        let mut code =
            catalog::read(Path::new("data/songs/848e1995c2c331f2bbd37d1b0b7ce281.js")).unwrap();
        let ctx = egui::Context::default();
        let mut editor = CodeEditor::default();
        let mut times = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            black_box(editor_frame(&ctx, &mut editor, &mut code, vec![]));
            times.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let first = times[0];
        times[1..].sort_unstable_by(f64::total_cmp);
        println!(
            "{} source bytes, {} preview bytes: first frame {:.3} ms; warm median {:.3} ms",
            code.len(),
            code_preview(&code).len(),
            first,
            times[50],
        );
        assert!(editor.guarded(&code));
    }

    #[derive(Default)]
    struct MemoryStorage(String);

    impl eframe::Storage for MemoryStorage {
        fn get_string(&self, _key: &str) -> Option<String> {
            Some(self.0.clone())
        }
        fn set_string(&mut self, _key: &str, value: String) {
            self.0 = value;
        }
        fn flush(&mut self) {}
    }

    #[test]
    fn restores_projects_saved_with_integer_sample_rates() {
        #[derive(Serialize)]
        struct OldSong {
            title: String,
            author: String,
            code: String,
            mode: Mode,
            rate: u32,
        }
        #[derive(Serialize)]
        struct OldProject {
            rate: u32,
            saved: Vec<OldSong>,
        }
        let mut storage = MemoryStorage::default();
        eframe::set_value(
            &mut storage,
            STORAGE_KEY,
            &OldProject {
                rate: 11_025,
                saved: vec![OldSong {
                    title: "Saved song".into(),
                    author: "Me".into(),
                    code: "t".into(),
                    mode: Mode::Bytebeat,
                    rate: 8_000,
                }],
            },
        );
        let project: Project = eframe::get_value(&storage, STORAGE_KEY)
            .expect("integer-rate projects must remain readable");
        assert_eq!(project.rate, 11_025.0);
        assert_eq!(project.saved[0].title, "Saved song");
        assert_eq!(project.saved[0].rate, 8_000.0);
    }
}
