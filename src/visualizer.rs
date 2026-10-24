use std::sync::{Arc, Mutex};

use eframe::egui::{self, Color32, FontId, Stroke, Vec2};
use rustfft::{Fft, FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};

use crate::shader_art::{self, AudioHistory, Draw, Renderer};

const FFT_SIZE: usize = 2048;
const GREEN: [u8; 3] = [100, 224, 150];
const BLUE: [u8; 3] = [99, 184, 241];

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum GraphKind {
    Waveform,
    Spectrum,
    Bits,
    Shader(usize),
}

impl GraphKind {
    fn label(self) -> &'static str {
        match self {
            Self::Waveform => "Waveform",
            Self::Spectrum => "FFT spectrum",
            Self::Bits => "8-bit pattern",
            Self::Shader(index) => shader_art::presets()
                .get(index)
                .map_or("Unavailable shader", |preset| preset.name.as_str()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
enum Channel {
    Left,
    Right,
    Mix,
}

impl Channel {
    fn label(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
            Self::Mix => "Mix",
        }
    }

    fn sample(self, frame: [f32; 2]) -> f32 {
        match self {
            Self::Left => frame[0],
            Self::Right => frame[1],
            Self::Mix => (frame[0] + frame[1]) * 0.5,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
            Self::Mix => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    kind: GraphKind,
    channel: Channel,
    color: [u8; 3],
    opacity: u8,
}

impl Graph {
    fn new(kind: GraphKind, channel: Channel, color: [u8; 3]) -> Self {
        Self {
            kind,
            channel,
            color,
            opacity: 255,
        }
    }

    fn color(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(self.color[0], self.color[1], self.color[2], self.opacity)
    }
}

pub fn default_graphs() -> Vec<Graph> {
    let mut graphs = spectrum_graphs();
    graphs.extend(waveform_graphs());
    graphs
}

pub fn waveform_graphs() -> Vec<Graph> {
    vec![
        Graph::new(GraphKind::Waveform, Channel::Left, GREEN),
        Graph::new(GraphKind::Waveform, Channel::Right, BLUE),
    ]
}

pub fn spectrum_graphs() -> Vec<Graph> {
    vec![Graph::new(GraphKind::Spectrum, Channel::Mix, BLUE)]
}

pub struct Visualizer {
    fft: Arc<dyn Fft<f32>>,
    zoom: f32,
    renderer: Option<Arc<Mutex<Renderer>>>,
    renderer_error: Option<String>,
    histories: [Arc<AudioHistory>; 3],
    history_time: [Option<u64>; 3],
    revision: u64,
    generation: u64,
}

impl Default for Visualizer {
    fn default() -> Self {
        Self {
            fft: FftPlanner::new().plan_fft_forward(FFT_SIZE),
            zoom: 1.0,
            renderer: None,
            renderer_error: None,
            histories: std::array::from_fn(|_| Arc::new(AudioHistory::default())),
            history_time: [None; 3],
            revision: 0,
            generation: 0,
        }
    }
}

fn graph_menu(ui: &mut egui::Ui, graphs: &mut Vec<Graph>) {
    ui.label("Later layers draw over earlier ones.");
    let mut remove = None;
    egui::ScrollArea::vertical()
        .max_height(280.0)
        .show(ui, |ui| {
            for (index, graph) in graphs.iter_mut().enumerate() {
                ui.push_id(index, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("-").on_hover_text("Remove graph").clicked() {
                            remove = Some(index);
                        }
                        let was_shader = matches!(graph.kind, GraphKind::Shader(_));
                        egui::ComboBox::from_id_salt("kind")
                            .selected_text(graph.kind.label())
                            .height(300.0)
                            .show_ui(ui, |ui| {
                                ui.label("Signal graphs");
                                for kind in
                                    [GraphKind::Waveform, GraphKind::Spectrum, GraphKind::Bits]
                                {
                                    ui.selectable_value(&mut graph.kind, kind, kind.label());
                                }
                                ui.separator();
                                ui.label("Shader art (CC0)");
                                for (index, preset) in shader_art::presets().iter().enumerate() {
                                    if ui
                                        .selectable_value(
                                            &mut graph.kind,
                                            GraphKind::Shader(index),
                                            &preset.name,
                                        )
                                        .changed()
                                        && !was_shader
                                    {
                                        graph.color = [255; 3];
                                        graph.opacity = 255;
                                    }
                                }
                            });
                        egui::ComboBox::from_id_salt("channel")
                            .selected_text(graph.channel.label())
                            .show_ui(ui, |ui| {
                                for channel in [Channel::Left, Channel::Right, Channel::Mix] {
                                    ui.selectable_value(
                                        &mut graph.channel,
                                        channel,
                                        channel.label(),
                                    );
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        ui.label("Color");
                        ui.color_edit_button_srgb(&mut graph.color);
                        ui.add(egui::Slider::new(&mut graph.opacity, 0..=255).text("Opacity"))
                            .on_hover_text("0 = transparent, 255 = opaque");
                    });
                    if let GraphKind::Shader(index) = graph.kind
                        && let Some(preset) = shader_art::presets().get(index)
                    {
                        ui.hyperlink_to(
                            format!("{} by {} ({})", preset.name, preset.author, preset.license),
                            &preset.source,
                        );
                    }
                    ui.separator();
                });
            }
        });
    if let Some(index) = remove {
        graphs.remove(index);
    }
    if ui.button("+").on_hover_text("Add graph").clicked() {
        let mut graph = Graph::new(GraphKind::Bits, Channel::Left, [130, 110, 245]);
        graph.opacity = 96;
        graphs.push(graph);
    }
}

fn visible_frames(frames: &[[f32; 2]], zoom: f32) -> &[[f32; 2]] {
    let count = ((frames.len() as f32 / zoom) as usize)
        .max(2)
        .min(frames.len());
    &frames[frames.len() - count..]
}

fn sample_byte(sample: f32) -> u8 {
    ((sample.clamp(-1.0, 1.0) + 1.0) * 128.0).floor().min(255.0) as u8
}

fn bit_mesh(rect: egui::Rect, frames: &[[f32; 2]], graph: &Graph) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    // At most one sample per screen column, regardless of audio history size.
    let columns = (rect.width().ceil() as usize).min(frames.len());
    for column in 0..columns {
        let sample = sample_byte(
            graph
                .channel
                .sample(frames[(column + 1) * frames.len() / columns - 1]),
        );
        for bit in 0..8 {
            if sample & (1 << bit) != 0 {
                mesh.add_colored_rect(
                    egui::Rect::from_min_max(
                        egui::pos2(
                            rect.left() + column as f32 / columns as f32 * rect.width(),
                            rect.bottom() - (bit + 1) as f32 / 8.0 * rect.height(),
                        ),
                        egui::pos2(
                            rect.left() + (column + 1) as f32 / columns as f32 * rect.width(),
                            rect.bottom() - bit as f32 / 8.0 * rect.height(),
                        ),
                    ),
                    graph.color(),
                );
            }
        }
    }
    mesh
}

impl Visualizer {
    pub fn new(gl: Option<&Arc<eframe::glow::Context>>) -> Self {
        let mut visualizer = Self::default();
        if let Some(gl) = gl {
            match Renderer::new(gl.clone()) {
                Ok(renderer) => visualizer.renderer = Some(Arc::new(Mutex::new(renderer))),
                Err(error) => {
                    log::error!("Shader-art renderer: {error}");
                    visualizer.renderer_error = Some(error);
                }
            }
        }
        visualizer
    }

    pub fn reset_history(&mut self) {
        self.histories = std::array::from_fn(|_| Arc::new(AudioHistory::default()));
        self.history_time = [None; 3];
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn set_generation(&mut self, generation: u64) {
        if self.generation != generation {
            self.reset_history();
            self.generation = generation;
        }
    }

    pub fn destroy(&mut self) {
        if let Some(renderer) = self.renderer.take() {
            renderer
                .lock()
                .expect("shader renderer mutex poisoned")
                .destroy();
        }
    }

    fn update_histories(&mut self, graphs: &[Graph], frames: &[[f32; 2]], time: f64) {
        if frames.len() < FFT_SIZE {
            return;
        }
        let key = time.to_bits();
        let mut changed = false;
        for channel in [Channel::Left, Channel::Right, Channel::Mix] {
            if self.history_time[channel.index()] == Some(key)
                || !graphs.iter().any(|graph| {
                    matches!(graph.kind, GraphKind::Shader(_))
                        && graph.channel == channel
                        && graph.opacity > 0
                })
            {
                continue;
            }
            let frames = &frames[frames.len() - FFT_SIZE..];
            let mut bins: Vec<_> = frames
                .iter()
                .enumerate()
                .map(|(i, frame)| {
                    let window =
                        0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos();
                    Complex::new(channel.sample(*frame) * window, 0.0)
                })
                .collect();
            self.fft.process(&mut bins);
            let magnitudes: Vec<_> = bins[..FFT_SIZE / 2]
                .iter()
                .map(|bin| bin.norm() / 512.0)
                .collect();
            Arc::make_mut(&mut self.histories[channel.index()]).push(
                &magnitudes,
                frames.iter().map(|frame| channel.sample(*frame)),
            );
            self.history_time[channel.index()] = Some(key);
            changed = true;
        }
        if changed {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        graphs: &mut Vec<Graph>,
        frames: &[[f32; 2]],
        rate: u32,
        time: f64,
    ) -> bool {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                egui::RichText::new("VISUALIZER")
                    .strong()
                    .color(Color32::from_rgb(GREEN[0], GREEN[1], GREEN[2])),
            );
            ui.menu_button("Graphs", |ui| graph_menu(ui, graphs));
            ui.add(
                egui::Slider::new(&mut self.zoom, 1.0..=16.0)
                    .text("Zoom")
                    .logarithmic(true),
            )
            .on_hover_text("Visible waveform and 8-bit pattern window");
        });
        self.update_histories(graphs, frames, time);
        if graphs
            .iter()
            .any(|graph| matches!(graph.kind, GraphKind::Shader(_)))
        {
            let errors = if let Some(renderer) = &self.renderer {
                let renderer = renderer.lock().expect("shader renderer mutex poisoned");
                graphs
                    .iter()
                    .filter_map(|graph| match graph.kind {
                        GraphKind::Shader(index) if graph.opacity > 0 => {
                            renderer.error(index).map(str::to_owned)
                        }
                        _ => None,
                    })
                    .collect()
            } else {
                vec![
                    self.renderer_error
                        .clone()
                        .unwrap_or_else(|| "Shader art requires the OpenGL renderer.".into()),
                ]
            };
            for error in errors {
                ui.colored_label(Color32::from_rgb(255, 130, 120), error);
            }
        }
        let height = (ui.available_height() * 0.36).clamp(150.0, 280.0);
        let (response, painter) = ui.allocate_painter(
            Vec2::new(ui.available_width(), height),
            egui::Sense::click(),
        );
        let rect = response.rect;
        let painter = painter.with_clip_rect(rect);
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
        let visible = visible_frames(frames, self.zoom);
        for graph in graphs.iter().filter(|graph| graph.opacity > 0) {
            match graph.kind {
                GraphKind::Spectrum if frames.len() >= FFT_SIZE && rate > 40 => {
                    self.spectrum(&painter, rect, frames, rate, graph);
                }
                GraphKind::Waveform if !visible.is_empty() => {
                    let points = visible
                        .iter()
                        .enumerate()
                        .map(|(i, frame)| {
                            egui::pos2(
                                rect.left()
                                    + i as f32 / (visible.len() - 1).max(1) as f32 * rect.width(),
                                rect.center().y
                                    - graph.channel.sample(*frame) * rect.height() * 0.44,
                            )
                        })
                        .collect();
                    painter.add(egui::Shape::line(
                        points,
                        Stroke::new(1.0_f32, graph.color()),
                    ));
                }
                GraphKind::Bits => {
                    painter.add(bit_mesh(rect, visible, graph));
                }
                GraphKind::Shader(index) => {
                    if shader_art::presets().get(index).is_none() {
                        painter.text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            format!("Unavailable shader preset: {index}"),
                            FontId::monospace(14.0),
                            Color32::RED,
                        );
                    } else if let Some(renderer) = &self.renderer {
                        let mouse = response.hover_pos().map_or([0.0; 2], |pos| {
                            [
                                (pos.x - rect.left()) / rect.width() * 2.0 - 1.0,
                                1.0 - (pos.y - rect.top()) / rect.height() * 2.0,
                            ]
                        });
                        painter.add(egui::PaintCallback {
                            rect,
                            callback: Arc::new(Renderer::callback(
                                renderer.clone(),
                                Draw {
                                    preset: index,
                                    channel: graph.channel.index(),
                                    tint: graph.color,
                                    opacity: graph.opacity,
                                    time: time as f32,
                                    mouse,
                                },
                                self.histories[graph.channel.index()].clone(),
                                self.revision,
                            )),
                        });
                    }
                }
                _ => {}
            }
        }

        let message = if graphs.is_empty() {
            Some("No graphs - use Graphs > + to add one")
        } else if frames.is_empty() {
            Some("Press Play to generate audio")
        } else {
            None
        };
        if let Some(message) = message {
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                message,
                FontId::monospace(14.0),
                Color32::GRAY,
            );
        }
        response.clicked()
    }
}

impl Visualizer {
    fn spectrum(
        &self,
        painter: &egui::Painter,
        rect: egui::Rect,
        frames: &[[f32; 2]],
        rate: u32,
        graph: &Graph,
    ) {
        let mut bins: Vec<_> = frames[frames.len() - FFT_SIZE..]
            .iter()
            .enumerate()
            .map(|(i, frame)| {
                let window = 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos();
                Complex::new(graph.channel.sample(*frame) * window, 0.0)
            })
            .collect();
        self.fft.process(&mut bins);
        let min_hz = 20.0_f32;
        let max_hz = rate as f32 / 2.0;
        let points = (1..FFT_SIZE / 2)
            .filter_map(|i| {
                let hz = i as f32 * rate as f32 / FFT_SIZE as f32;
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
        painter.add(egui::Shape::line(
            points,
            Stroke::new(1.5_f32, graph.color()),
        ));
        for hz in [100.0_f32, 1000.0, 10000.0] {
            if hz < max_hz {
                let x = rect.left() + (hz / min_hz).ln() / (max_hz / min_hz).ln() * rect.width();
                painter.text(
                    egui::pos2(x, rect.bottom() - 3.0),
                    egui::Align2::CENTER_BOTTOM,
                    format!("{} Hz", hz as u32),
                    FontId::monospace(10.0),
                    graph.color(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu_frame(
        ctx: &egui::Context,
        graphs: &mut Vec<Graph>,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| graph_menu(ui, graphs));
            },
        )
    }

    fn click_label(ctx: &egui::Context, graphs: &mut Vec<Graph>, label: &str) {
        let output = menu_frame(ctx, graphs, vec![]);
        let pos = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + egui::vec2(2.0, 2.0))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing graph control: {label}"));
        for pressed in [true, false] {
            menu_frame(
                ctx,
                graphs,
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
        }
    }

    #[test]
    fn menu_adds_and_removes_graphs_including_last_graph() {
        let ctx = egui::Context::default();
        let mut graphs = Vec::new();
        click_label(&ctx, &mut graphs, "+");
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0].kind, GraphKind::Bits);
        assert!(graphs[0].opacity > 0 && graphs[0].opacity < 255);
        click_label(&ctx, &mut graphs, "+");
        assert_eq!(graphs.len(), 2);
        graphs[1].color = [1, 2, 3];
        click_label(&ctx, &mut graphs, "-");
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0].color, [1, 2, 3]);
        click_label(&ctx, &mut graphs, "-");
        assert!(graphs.is_empty());
        click_label(&ctx, &mut graphs, "+");
        assert_eq!(graphs.len(), 1);
    }

    #[test]
    fn menu_changes_graph_kind_and_channel() {
        let ctx = egui::Context::default();
        let mut graphs = waveform_graphs();
        graphs.truncate(1);
        click_label(&ctx, &mut graphs, "Waveform");
        click_label(&ctx, &mut graphs, "8-bit pattern");
        assert_eq!(graphs[0].kind, GraphKind::Bits);
        click_label(&ctx, &mut graphs, "Left");
        click_label(&ctx, &mut graphs, "Right");
        assert_eq!(graphs[0].channel, Channel::Right);
        click_label(&ctx, &mut graphs, "8-bit pattern");
        click_label(&ctx, &mut graphs, "FFT spectrum");
        assert_eq!(graphs[0].kind, GraphKind::Spectrum);
    }

    #[test]
    fn bit_pattern_recovers_every_unsigned_byte_and_clips_float_endpoints() {
        for byte in 0..=255 {
            assert_eq!(sample_byte(byte as f32 / 128.0 - 1.0), byte as u8);
        }
        assert_eq!(sample_byte(-2.0), 0);
        assert_eq!(sample_byte(0.0), 128);
        assert_eq!(sample_byte(1.0), 255);
        assert_eq!(sample_byte(2.0), 255);
    }

    #[test]
    fn bit_mesh_uses_selected_channel_color_alpha_and_bit_order() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(2.0, 80.0));
        let mut graph = Graph::new(GraphKind::Bits, Channel::Left, [200, 100, 50]);
        graph.opacity = 64;
        let frames = [[1.0 / 128.0 - 1.0, -1.0], [0.0, -1.0]];
        let mesh = bit_mesh(rect, &frames, &graph);
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.indices.len(), 12);
        assert_eq!(mesh.calc_bounds().left(), rect.left());
        assert_eq!(mesh.calc_bounds().right(), rect.right());
        let first: Vec<_> = mesh.vertices[..4].iter().map(|v| v.pos.y).collect();
        assert!(first.iter().all(|y| *y >= rect.bottom() - 10.0));
        let second: Vec<_> = mesh.vertices[4..].iter().map(|v| v.pos.y).collect();
        assert!(second.iter().all(|y| *y <= rect.top() + 10.0));
        assert!(mesh.vertices.iter().all(|v| v.color == graph.color()));
        assert_eq!(graph.color().a(), 64);
        graph.channel = Channel::Right;
        assert!(bit_mesh(rect, &frames, &graph).vertices.is_empty());
        graph.channel = Channel::Mix;
        assert_eq!(graph.channel.sample([1.0, -1.0]), 0.0);
        assert!(!bit_mesh(rect, &frames, &graph).vertices.is_empty());
    }

    #[test]
    fn bit_mesh_is_bounded_and_zoom_uses_latest_frames() {
        let frames = vec![[1.0; 2]; FFT_SIZE];
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(100.0, 80.0));
        let graph = Graph::new(GraphKind::Bits, Channel::Left, GREEN);
        let mesh = bit_mesh(rect, &frames, &graph);
        assert_eq!(mesh.vertices.len(), 100 * 8 * 4);
        assert!(mesh.vertices.iter().all(|v| rect.contains(v.pos)));
        assert!(bit_mesh(rect, &[], &graph).vertices.is_empty());
        assert_eq!(visible_frames(&frames, 16.0).len(), FFT_SIZE / 16);
        let frames = [[-1.0; 2], [0.0; 2], [1.0; 2]];
        assert_eq!(visible_frames(&frames, 16.0), &frames[1..]);
        assert_eq!(visible_frames(&frames[..1], 16.0), &frames[..1]);
        assert!(visible_frames(&[], 1.0).is_empty());
    }

    #[test]
    fn all_graphs_render_empty_partial_and_full_history_without_changing_samples() {
        let ctx = egui::Context::default();
        let mut visualizer = Visualizer::default();
        let mut graphs = default_graphs();
        graphs.push(Graph::new(GraphKind::Bits, Channel::Left, GREEN));
        let frames: Vec<_> = (0..FFT_SIZE)
            .map(|i| {
                let phase = i as f32 * std::f32::consts::TAU / 48.0;
                [phase.sin(), phase.cos()]
            })
            .collect();
        let original = frames.clone();
        for count in [0, 1, 128, FFT_SIZE] {
            for rate in [0, 48_000] {
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        visualizer.show(ui, &mut graphs, &frames[..count], rate, 0.0);
                    });
                });
                let primitives = ctx.tessellate(output.shapes, output.pixels_per_point);
                assert!(!primitives.is_empty());
            }
        }
        assert_eq!(frames, original);
    }

    #[test]
    fn shader_layers_roundtrip_every_preset_and_preserve_tint_and_opacity() {
        let graphs: Vec<_> = shader_art::presets()
            .iter()
            .enumerate()
            .map(|(index, preset)| {
                let mut graph = Graph::new(GraphKind::Shader(index), Channel::Mix, [12, 34, 56]);
                graph.opacity = 87;
                assert_eq!(graph.kind.label(), preset.name);
                graph
            })
            .collect();
        let json = serde_json::to_string(&graphs).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Graph>>(&json).unwrap(), graphs);
    }

    #[test]
    fn shader_histories_do_not_advance_while_paused_and_reset_on_audio_generation() {
        let mut visualizer = Visualizer::default();
        let graph = Graph::new(GraphKind::Shader(0), Channel::Right, [255; 3]);
        let frames = vec![[0.25, 0.5]; FFT_SIZE];
        visualizer.update_histories(std::slice::from_ref(&graph), &frames, 1.0);
        let revision = visualizer.revision;
        assert_eq!(
            visualizer.history_time,
            [None, Some(1.0_f64.to_bits()), None]
        );
        visualizer.update_histories(std::slice::from_ref(&graph), &frames, 1.0);
        assert_eq!(visualizer.revision, revision);
        visualizer.set_generation(1);
        assert_eq!(visualizer.history_time, [None; 3]);
        assert_ne!(visualizer.revision, revision);
        visualizer.update_histories(std::slice::from_ref(&graph), &frames, 1.0);
        assert_eq!(visualizer.history_time[1], Some(1.0_f64.to_bits()));
        let revision = visualizer.revision;
        visualizer.set_generation(1);
        assert_eq!(visualizer.revision, revision);
        let mut hidden = graph;
        hidden.opacity = 0;
        visualizer.update_histories(&[hidden], &frames, 2.0);
        assert_eq!(visualizer.revision, revision);
    }

    #[test]
    fn shader_without_gpu_reports_error_instead_of_silently_drawing_nothing() {
        let ctx = egui::Context::default();
        let mut visualizer = Visualizer::default();
        let mut graphs = vec![Graph::new(GraphKind::Shader(0), Channel::Left, [255; 3])];
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                visualizer.show(ui, &mut graphs, &[], 48_000, 0.0);
            });
        });
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Shader art requires the OpenGL renderer."
        )));
    }

    #[test]
    fn menu_selects_original_shader_art_with_default_colors() {
        let ctx = egui::Context::default();
        let mut graphs = vec![Graph::new(GraphKind::Waveform, Channel::Left, GREEN)];
        click_label(&ctx, &mut graphs, "Waveform");
        click_label(&ctx, &mut graphs, "blorp");
        assert_eq!(graphs[0].kind, GraphKind::Shader(0));
        assert_eq!(graphs[0].color, [255; 3]);
        assert_eq!(graphs[0].opacity, 255);
        graphs[0].color = [10, 20, 30];
        graphs[0].opacity = 100;
        click_label(&ctx, &mut graphs, "blorp");
        click_label(&ctx, &mut graphs, "seaqyuk");
        assert_eq!(graphs[0].kind, GraphKind::Shader(1));
        assert_eq!(graphs[0].color, [10, 20, 30]);
        assert_eq!(graphs[0].opacity, 100);
    }
}
