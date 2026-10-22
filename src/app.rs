use std::{
    path::PathBuf,
    sync::{Arc, mpsc::TryRecvError},
    time::Duration,
};

use eframe::egui::{self, Color32, FontId, RichText, Stroke, Vec2};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

use crate::{
    audio::{Audio, Command},
    catalog,
    engine::Mode,
    songs::{self, Song},
    upstream::{self, Download, Event},
};

const GREEN: Color32 = Color32::from_rgb(100, 224, 150);
const BLUE: Color32 = Color32::from_rgb(99, 184, 241);
const STORAGE_KEY: &str = "bytebeat-project-v1";

#[derive(Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
enum Scope {
    #[default]
    Waveform,
    Spectrum,
    Combined,
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
struct Project {
    code: String,
    title: String,
    mode: Mode,
    rate: f64,
    volume: f32,
    scope: Scope,
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
            scope: Scope::Combined,
            saved: vec![],
        }
    }
}

pub struct Composer {
    project: Project,
    audio: Audio,
    demos: Vec<Song>,
    search: String,
    only_saved: bool,
    loaded: Option<(String, Mode, f64)>,
    fft: Arc<dyn Fft<f32>>,
    zoom: f32,
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
            loaded: None,
            fft: FftPlanner::new().plan_fft_forward(2048),
            zoom: 1.0,
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
        let query = self.search.to_lowercase();
        let mut selected = None;
        let mut remove = None;
        let filtered: Vec<_> = self
            .demos
            .iter()
            .chain(self.project.saved.iter())
            .enumerate()
            .filter(|(i, song)| {
                (!self.only_saved || *i >= self.demos.len())
                    && (query.is_empty()
                        || format!("{} {} {}", song.title, song.author, song.code)
                            .to_lowercase()
                            .contains(&query))
            })
            .collect();
        ui.label(format!("{} songs", filtered.len()));
        egui::ScrollArea::vertical().show_rows(ui, 132.0, filtered.len(), |ui, rows| {
            for row in rows {
                let (i, song) = filtered[row];
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
        }
        if let Some(song) = selected {
            self.project.code = song.code;
            self.project.title = song.title;
            self.project.mode = song.mode;
            self.project.rate = song.rate;
            self.load(true);
        }
    }

    fn visualizer(&mut self, ui: &mut egui::Ui, frames: &[[f32; 2]], rate: u32) -> bool {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("OSCILLOSCOPE / SPECTRUM")
                    .strong()
                    .color(GREEN),
            );
            for (mode, label) in [
                (Scope::Waveform, "Waveform"),
                (Scope::Spectrum, "FFT"),
                (Scope::Combined, "Combined"),
            ] {
                ui.selectable_value(&mut self.project.scope, mode, label);
            }
            ui.add(
                egui::Slider::new(&mut self.zoom, 1.0..=16.0)
                    .text("Zoom")
                    .logarithmic(true),
            );
        });
        let height = (ui.available_height() * 0.36).clamp(150.0, 280.0);
        let (response, painter) = ui.allocate_painter(
            Vec2::new(ui.available_width(), height),
            egui::Sense::click(),
        );
        let rect = response.rect;
        painter.rect_filled(rect, 3.0, Color32::from_rgb(10, 15, 17));
        for x in 0..=10 {
            let x = rect.left() + rect.width() * x as f32 / 10.0;
            painter.line_segment(
                [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                Stroke::new(1.0_f32, Color32::from_gray(35)),
            );
        }
        for y in 0..=4 {
            let y = rect.top() + rect.height() * y as f32 / 4.0;
            painter.line_segment(
                [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
                Stroke::new(1.0_f32, Color32::from_gray(35)),
            );
        }
        if self.project.scope != Scope::Waveform && frames.len() >= 2048 && rate > 0 {
            let mut bins: Vec<_> = frames[frames.len() - 2048..]
                .iter()
                .enumerate()
                .map(|(i, frame)| {
                    let window = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / 2048.0).cos();
                    Complex::new((frame[0] + frame[1]) * 0.5 * window, 0.0)
                })
                .collect();
            self.fft.process(&mut bins);
            let min_hz = 20.0_f32;
            let max_hz = rate as f32 / 2.0;
            let points: Vec<_> = (1..1024)
                .filter_map(|i| {
                    let hz = i as f32 * rate as f32 / 2048.0;
                    if hz < min_hz {
                        return None;
                    }
                    let x = (hz / min_hz).ln() / (max_hz / min_hz).ln();
                    let db = 20.0 * (bins[i].norm() / 512.0).max(1e-6).log10();
                    let y = ((db + 100.0) / 100.0).clamp(0.0, 1.0);
                    Some(egui::pos2(
                        rect.left() + x * rect.width(),
                        rect.bottom() - y * rect.height(),
                    ))
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(1.5_f32, BLUE)));
            for hz in [100.0_f32, 1000.0, 10000.0] {
                if hz < max_hz {
                    let x =
                        rect.left() + (hz / min_hz).ln() / (max_hz / min_hz).ln() * rect.width();
                    painter.text(
                        egui::pos2(x, rect.bottom() - 3.0),
                        egui::Align2::CENTER_BOTTOM,
                        format!("{} Hz", hz as u32),
                        FontId::monospace(10.0),
                        BLUE,
                    );
                }
            }
        }
        if self.project.scope != Scope::Spectrum && !frames.is_empty() {
            let count = ((frames.len() as f32 / self.zoom) as usize)
                .max(2)
                .min(frames.len());
            let start = frames.len() - count;
            for (channel, color) in [(0, GREEN), (1, BLUE)] {
                let points = frames[start..]
                    .iter()
                    .enumerate()
                    .map(|(i, frame)| {
                        egui::pos2(
                            rect.left() + i as f32 / (count - 1).max(1) as f32 * rect.width(),
                            rect.center().y - frame[channel] * rect.height() * 0.44,
                        )
                    })
                    .collect();
                painter.add(egui::Shape::line(points, Stroke::new(1.0_f32, color)));
            }
        }
        if frames.is_empty() {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Press Play to generate audio",
                FontId::monospace(14.0),
                Color32::GRAY,
            );
        }
        response.clicked()
    }
}

impl eframe::App for Composer {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_catalog();
        let (playing, position, frames, rate, error) = {
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
            )
        };
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
            if self.visualizer(ui, &frames, rate) {
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
            let editor_height = (ui.available_height() - 4.0).max(100.0);
            egui::ScrollArea::both().max_height(editor_height).show(ui, |ui| {
                ui.add_sized([ui.available_width(), editor_height],
                    egui::TextEdit::multiline(&mut self.project.code)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .lock_focus(true));
            });
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
