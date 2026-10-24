use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use eframe::{
    egui,
    egui_glow::CallbackFn,
    glow::{self, HasContext},
};
use serde::Deserialize;

const BINS: usize = 1024;
const HISTORY: usize = 240;

#[derive(Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub author: String,
    pub source: String,
    pub license: String,
    settings: Settings,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Settings {
    num: i32,
    mode: String,
    background_color: [f32; 4],
    shader: String,
}

pub fn presets() -> &'static [Preset] {
    static PRESETS: OnceLock<Vec<Preset>> = OnceLock::new();
    PRESETS.get_or_init(|| {
        serde_json::from_str(include_str!("../assets/vertexshaderart.json"))
            .expect("embedded shader-art catalog must be valid")
    })
}

#[derive(Clone)]
pub struct AudioHistory {
    sound: Vec<u8>,
    volume: Vec<u8>,
    smoothed: Vec<f32>,
}

impl Default for AudioHistory {
    fn default() -> Self {
        Self {
            sound: vec![0; BINS * HISTORY * 4],
            volume: vec![0; 4 * HISTORY * 4],
            smoothed: vec![0.0; BINS],
        }
    }
}

impl AudioHistory {
    pub fn push(&mut self, magnitudes: &[f32], samples: impl Iterator<Item = f32>) {
        self.sound.copy_within(..BINS * (HISTORY - 1) * 4, BINS * 4);
        for (index, (previous, magnitude)) in self.smoothed.iter_mut().zip(magnitudes).enumerate() {
            *previous = *previous * 0.8 + magnitude * 0.2;
            let db = 20.0 * previous.max(1e-10).log10();
            let value = ((db + 100.0) / 70.0 * 255.0).clamp(0.0, 255.0) as u8;
            self.sound[index * 4..index * 4 + 4].fill(value);
        }
        let mut peak = 0.0_f32;
        let mut squares = 0.0;
        let mut difference = 0.0_f32;
        let mut previous = 0.0;
        let mut count = 0;
        for sample in samples {
            peak = peak.max(sample.abs());
            squares += sample * sample;
            difference = difference.max((sample - previous).abs());
            previous = sample;
            count += 1;
        }
        self.volume.copy_within(..4 * (HISTORY - 1) * 4, 4 * 4);
        let rms = (squares / count.max(1) as f32).sqrt();
        let max_frequency = self.sound[..BINS * 4].iter().copied().max().unwrap_or(0);
        for (index, value) in [
            (peak * 255.0).clamp(0.0, 255.0) as u8,
            (rms * 255.0).clamp(0.0, 255.0) as u8,
            (difference * 127.0).clamp(0.0, 255.0) as u8,
            max_frequency,
        ]
        .into_iter()
        .enumerate()
        {
            self.volume[index * 4..index * 4 + 4].fill(value);
        }
    }
}

pub struct Draw {
    pub preset: usize,
    pub channel: usize,
    pub tint: [u8; 3],
    pub opacity: u8,
    pub time: f32,
    pub mouse: [f32; 2],
}

struct Textures {
    sound: glow::Texture,
    volume: glow::Texture,
    touch: glow::Texture,
    revision: Option<u64>,
}

struct Target {
    framebuffer: glow::Framebuffer,
    color: glow::Texture,
    depth: glow::Renderbuffer,
    width: i32,
    height: i32,
}

pub struct Renderer {
    gl: Arc<glow::Context>,
    vao: glow::VertexArray,
    composite: glow::Program,
    programs: HashMap<usize, Result<glow::Program, String>>,
    textures: HashMap<usize, Textures>,
    target: Option<Target>,
    errors: HashMap<usize, String>,
}

fn shader_source(preset: &Preset, embedded: bool) -> String {
    let version = if embedded {
        "#version 300 es\nprecision highp float;\n"
    } else {
        "#version 150\n"
    };
    format!(
        "{version}\
         float vertexId;\n\
         #define texture2D texture\n\
         #define inverse vsa_inverse\n\
         #define transpose vsa_transpose\n\
         uniform vec2 mouse;\n\
         uniform vec2 resolution;\n\
         uniform vec4 background;\n\
         uniform float time;\n\
         uniform float vertexCount;\n\
         uniform sampler2D volume;\n\
         uniform sampler2D sound;\n\
         uniform sampler2D floatSound;\n\
         uniform sampler2D touch;\n\
         uniform vec2 soundRes;\n\
         out vec4 v_color;\n\
         #define main vsa_main\n\
         {}\n\
         #undef main\n\
         void main() {{ vertexId = float(gl_VertexID); gl_PointSize = 1.0; vsa_main(); gl_PointSize = max(1.0, gl_PointSize); }}\n",
        preset.settings.shader
    )
}

fn compile_program(
    gl: &glow::Context,
    vertex: &str,
    fragment: &str,
) -> Result<glow::Program, String> {
    // Called only with the application's OpenGL context current.
    unsafe {
        let program = gl.create_program()?;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = match gl.create_shader(kind) {
                Ok(shader) => shader,
                Err(error) => {
                    gl.delete_program(program);
                    return Err(error);
                }
            };
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let error = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                gl.delete_program(program);
                return Err(error);
            }
            gl.attach_shader(program, shader);
            gl.delete_shader(shader);
        }
        gl.link_program(program);
        if !gl.get_program_link_status(program) {
            let error = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(error);
        }
        Ok(program)
    }
}

fn texture(gl: &glow::Context, width: i32, height: i32) -> Result<glow::Texture, String> {
    unsafe {
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MIN_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_MAG_FILTER,
            glow::LINEAR as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_S,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_parameter_i32(
            glow::TEXTURE_2D,
            glow::TEXTURE_WRAP_T,
            glow::CLAMP_TO_EDGE as i32,
        );
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGBA8 as i32,
            width,
            height,
            0,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(None),
        );
        Ok(texture)
    }
}

impl Textures {
    fn new(gl: &glow::Context) -> Result<Self, String> {
        let sound = texture(gl, BINS as i32, HISTORY as i32)?;
        let volume = match texture(gl, 4, HISTORY as i32) {
            Ok(volume) => volume,
            Err(error) => {
                unsafe { gl.delete_texture(sound) };
                return Err(error);
            }
        };
        let touch = match texture(gl, 32, HISTORY as i32) {
            Ok(touch) => touch,
            Err(error) => {
                unsafe {
                    gl.delete_texture(sound);
                    gl.delete_texture(volume);
                }
                return Err(error);
            }
        };
        // These presets do not require touch interaction; provide a defined neutral texture.
        upload(gl, touch, 32, HISTORY as i32, &vec![0; 32 * HISTORY * 4]);
        Ok(Self {
            sound,
            volume,
            touch,
            revision: None,
        })
    }

    fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_texture(self.sound);
            gl.delete_texture(self.volume);
            gl.delete_texture(self.touch);
        }
    }
}

fn upload(gl: &glow::Context, texture: glow::Texture, width: i32, height: i32, data: &[u8]) {
    unsafe {
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        gl.tex_sub_image_2d(
            glow::TEXTURE_2D,
            0,
            0,
            0,
            width,
            height,
            glow::RGBA,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(data)),
        );
    }
}

impl Target {
    fn new(gl: &glow::Context, width: i32, height: i32) -> Result<Self, String> {
        let color = texture(gl, width, height)?;
        let framebuffer = match unsafe { gl.create_framebuffer() } {
            Ok(framebuffer) => framebuffer,
            Err(error) => {
                unsafe { gl.delete_texture(color) };
                return Err(error);
            }
        };
        let depth = match unsafe { gl.create_renderbuffer() } {
            Ok(depth) => depth,
            Err(error) => {
                unsafe {
                    gl.delete_texture(color);
                    gl.delete_framebuffer(framebuffer);
                }
                return Err(error);
            }
        };
        let target = Self {
            framebuffer,
            color,
            depth,
            width,
            height,
        };
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(framebuffer));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(color),
                0,
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, width, height);
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            if gl.check_framebuffer_status(glow::FRAMEBUFFER) != glow::FRAMEBUFFER_COMPLETE {
                target.destroy(gl);
                return Err("Shader-art framebuffer is incomplete".into());
            }
        }
        Ok(target)
    }

    fn destroy(self, gl: &glow::Context) {
        unsafe {
            gl.delete_framebuffer(self.framebuffer);
            gl.delete_texture(self.color);
            gl.delete_renderbuffer(self.depth);
        }
    }
}

impl Renderer {
    pub fn new(gl: Arc<glow::Context>) -> Result<Self, String> {
        let version = if gl.version().is_embedded {
            "#version 300 es\nprecision highp float;\n"
        } else {
            "#version 150\n"
        };
        let composite = compile_program(
            &gl,
            &format!(
                "{version}out vec2 uv;\nvoid main() {{
                    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
                    uv = p; gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
                }}"
            ),
            &format!(
                "{version}in vec2 uv; out vec4 color;
                 uniform sampler2D image; uniform vec4 tint;
                 void main() {{ vec4 c = texture(image, uv);
                    color = vec4(c.rgb * tint.rgb * tint.a, c.a * tint.a); }}"
            ),
        )?;
        let vao = match unsafe { gl.create_vertex_array() } {
            Ok(vao) => vao,
            Err(error) => {
                unsafe { gl.delete_program(composite) };
                return Err(error);
            }
        };
        Ok(Self {
            gl,
            vao,
            composite,
            programs: HashMap::new(),
            textures: HashMap::new(),
            target: None,
            errors: HashMap::new(),
        })
    }

    fn program(&mut self, index: usize) -> Result<glow::Program, String> {
        let preset = &presets()[index];
        let result = self.programs.entry(index).or_insert_with(|| {
            let version = if self.gl.version().is_embedded {
                "#version 300 es\nprecision highp float;\n"
            } else {
                "#version 150\n"
            };
            compile_program(
                &self.gl,
                &shader_source(preset, self.gl.version().is_embedded),
                &format!(
                    "{version}in vec4 v_color; out vec4 color;
                     void main() {{ color = v_color; }}"
                ),
            )
            .map_err(|error| format!("Shader art '{}' ({}): {error}", preset.name, preset.id))
        });
        result.clone()
    }

    pub fn callback(
        renderer: Arc<Mutex<Self>>,
        draw: Draw,
        history: Arc<AudioHistory>,
        revision: u64,
    ) -> CallbackFn {
        CallbackFn::new(move |info, painter| {
            let mut renderer = renderer.lock().expect("shader renderer mutex poisoned");
            match renderer.paint(info, painter.intermediate_fbo(), &draw, &history, revision) {
                Ok(()) => {
                    renderer.errors.remove(&draw.preset);
                }
                Err(error) => {
                    if renderer.errors.get(&draw.preset) != Some(&error) {
                        log::error!("{error}");
                    }
                    renderer.errors.insert(draw.preset, error);
                }
            }
        })
    }

    pub fn error(&self, preset: usize) -> Option<&str> {
        self.errors.get(&preset).map(String::as_str)
    }

    fn paint(
        &mut self,
        info: egui::PaintCallbackInfo,
        output: Option<glow::Framebuffer>,
        draw: &Draw,
        history: &AudioHistory,
        revision: u64,
    ) -> Result<(), String> {
        let result = self.paint_inner(&info, draw, history, revision);
        let gl = &self.gl;
        let viewport = info.viewport_in_pixels();
        // Offscreen depth and clears must never affect egui or other layers.
        unsafe {
            gl.bind_framebuffer(glow::FRAMEBUFFER, output);
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            gl.viewport(
                viewport.left_px,
                viewport.from_bottom_px,
                viewport.width_px,
                viewport.height_px,
            );
            gl.enable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            if !gl.version().is_embedded {
                gl.disable(glow::PROGRAM_POINT_SIZE);
            }
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            if result.is_ok()
                && let Some(target) = &self.target
            {
                gl.use_program(Some(self.composite));
                gl.bind_vertex_array(Some(self.vao));
                gl.active_texture(glow::TEXTURE0);
                gl.bind_texture(glow::TEXTURE_2D, Some(target.color));
                gl.uniform_1_i32(gl.get_uniform_location(self.composite, "image").as_ref(), 0);
                gl.uniform_4_f32(
                    gl.get_uniform_location(self.composite, "tint").as_ref(),
                    draw.tint[0] as f32 / 255.0,
                    draw.tint[1] as f32 / 255.0,
                    draw.tint[2] as f32 / 255.0,
                    draw.opacity as f32 / 255.0,
                );
                gl.draw_arrays(glow::TRIANGLES, 0, 3);
            }
            gl.bind_vertex_array(None);
            gl.use_program(None);
            gl.active_texture(glow::TEXTURE0);
        }
        let error = unsafe { gl.get_error() };
        result.and_then(|()| {
            if error == glow::NO_ERROR {
                Ok(())
            } else {
                Err(format!("Shader-art OpenGL error: {error:#x}"))
            }
        })
    }

    fn paint_inner(
        &mut self,
        info: &egui::PaintCallbackInfo,
        draw: &Draw,
        history: &AudioHistory,
        revision: u64,
    ) -> Result<(), String> {
        let program = self.program(draw.preset)?;
        let gl = &self.gl;
        let viewport = info.viewport_in_pixels();
        let width = viewport.width_px.max(1);
        let height = viewport.height_px.max(1);
        unsafe {
            gl.active_texture(glow::TEXTURE0);
            if self
                .target
                .as_ref()
                .is_none_or(|target| target.width != width || target.height != height)
            {
                if let Some(target) = self.target.take() {
                    target.destroy(gl);
                }
                self.target = Some(Target::new(gl, width, height)?);
            }
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.textures.entry(draw.channel)
            {
                entry.insert(Textures::new(gl)?);
            }
            let textures = self
                .textures
                .get_mut(&draw.channel)
                .expect("channel textures initialized");
            if textures.revision != Some(revision) {
                upload(
                    gl,
                    textures.sound,
                    BINS as i32,
                    HISTORY as i32,
                    &history.sound,
                );
                upload(gl, textures.volume, 4, HISTORY as i32, &history.volume);
                textures.revision = Some(revision);
            }
            let target = self.target.as_ref().expect("render target initialized");
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(target.framebuffer));
            gl.viewport(0, 0, width, height);
            gl.disable(glow::SCISSOR_TEST);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_mask(true);
            gl.depth_func(glow::LESS);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
            if !gl.version().is_embedded {
                gl.enable(glow::PROGRAM_POINT_SIZE);
            }
            let settings = &presets()[draw.preset].settings;
            let [r, g, b, a] = settings.background_color;
            gl.clear_color(r, g, b, a);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
            gl.bind_vertex_array(Some(self.vao));
            gl.use_program(Some(program));
            gl.uniform_1_f32(gl.get_uniform_location(program, "time").as_ref(), draw.time);
            gl.uniform_1_f32(
                gl.get_uniform_location(program, "vertexCount").as_ref(),
                settings.num as f32,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(program, "resolution").as_ref(),
                width as f32,
                height as f32,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(program, "soundRes").as_ref(),
                BINS as f32,
                HISTORY as f32,
            );
            gl.uniform_2_f32(
                gl.get_uniform_location(program, "mouse").as_ref(),
                draw.mouse[0],
                draw.mouse[1],
            );
            gl.uniform_4_f32(
                gl.get_uniform_location(program, "background").as_ref(),
                r,
                g,
                b,
                a,
            );
            for (unit, name, texture) in [
                (0, "sound", textures.sound),
                (1, "floatSound", textures.sound),
                (2, "volume", textures.volume),
                (3, "touch", textures.touch),
            ] {
                gl.active_texture(glow::TEXTURE0 + unit);
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), unit as i32);
            }
            let mode = match settings.mode.as_str() {
                "TRIANGLES" => glow::TRIANGLES,
                "LINES" => glow::LINES,
                "POINTS" => glow::POINTS,
                mode => return Err(format!("Unsupported shader primitive: {mode}")),
            };
            gl.draw_arrays(mode, 0, settings.num);
        }
        Ok(())
    }

    pub fn destroy(&mut self) {
        let gl = &self.gl;
        unsafe {
            for (_, program) in self.programs.drain() {
                if let Ok(program) = program {
                    gl.delete_program(program);
                }
            }
            gl.delete_program(self.composite);
            gl.delete_vertex_array(self.vao);
        }
        for (_, textures) in self.textures.drain() {
            textures.destroy(gl);
        }
        if let Some(target) = self.target.take() {
            target.destroy(gl);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_contains_all_35_original_cc0_presets() {
        let presets = presets();
        assert_eq!(presets.len(), 35);
        let mut ids = std::collections::HashSet::new();
        for preset in presets {
            assert!(ids.insert(&preset.id));
            assert_eq!(preset.author, "gman");
            assert_eq!(preset.license, "CC0-1.0");
            assert_eq!(
                preset.source,
                format!("https://www.vertexshaderart.com/art/{}", preset.id)
            );
            assert!((1..=100_000).contains(&preset.settings.num));
            assert!(matches!(
                preset.settings.mode.as_str(),
                "TRIANGLES" | "LINES" | "POINTS"
            ));
            assert!(preset.settings.shader.contains("void main"));
            assert!(
                preset
                    .settings
                    .background_color
                    .iter()
                    .all(|value| value.is_finite())
            );
        }
    }

    #[test]
    fn audio_history_scrolls_frequency_and_volume_rows() {
        let mut history = AudioHistory::default();
        assert!(history.sound.iter().all(|value| *value == 0));
        let mut magnitudes = vec![0.0; BINS];
        magnitudes[10] = 0.5;
        history.push(&magnitudes, [-0.5, 0.5].into_iter());
        assert!(history.sound[40] > 0);
        assert_eq!(history.sound[40], history.sound[43]);
        assert_eq!(history.volume[0], 127);
        let first_sound = history.sound[..BINS * 4].to_vec();
        let first_volume = history.volume[..16].to_vec();
        history.push(&vec![0.0; BINS], [0.0; 2].into_iter());
        assert_eq!(&history.sound[BINS * 4..BINS * 8], first_sound.as_slice());
        assert_eq!(&history.volume[16..32], first_volume.as_slice());
        assert!(
            history.sound[40] > 0,
            "frequency smoothing preserves some previous energy"
        );
        assert_eq!(history.volume[0], 0);
    }

    #[cfg(target_os = "macos")]
    mod native {
        use super::*;
        use std::ffi::{CString, c_void};

        #[link(name = "OpenGL", kind = "framework")]
        unsafe extern "C" {
            fn CGLChoosePixelFormat(
                attributes: *const u32,
                format: *mut *mut c_void,
                count: *mut i32,
            ) -> i32;
            fn CGLCreateContext(
                format: *mut c_void,
                share: *mut c_void,
                context: *mut *mut c_void,
            ) -> i32;
            fn CGLDestroyPixelFormat(format: *mut c_void) -> i32;
            fn CGLSetCurrentContext(context: *mut c_void) -> i32;
            fn CGLDestroyContext(context: *mut c_void) -> i32;
        }

        struct Context(*mut c_void);

        impl Context {
            fn new() -> Self {
                unsafe {
                    let attributes = [99, 0x3200, 73, 0];
                    let mut format = std::ptr::null_mut();
                    let mut count = 0;
                    assert_eq!(
                        CGLChoosePixelFormat(attributes.as_ptr(), &mut format, &mut count),
                        0
                    );
                    let mut context = std::ptr::null_mut();
                    let result = CGLCreateContext(format, std::ptr::null_mut(), &mut context);
                    CGLDestroyPixelFormat(format);
                    assert_eq!(result, 0, "creating an offscreen OpenGL context");
                    assert_eq!(CGLSetCurrentContext(context), 0);
                    Self(context)
                }
            }

            fn glow(&self) -> glow::Context {
                unsafe {
                    glow::Context::from_loader_function(|name| {
                        let name = CString::new(name).unwrap();
                        libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()).cast_const()
                    })
                }
            }
        }

        impl Drop for Context {
            fn drop(&mut self) {
                unsafe {
                    CGLSetCurrentContext(std::ptr::null_mut());
                    CGLDestroyContext(self.0);
                }
            }
        }

        fn info() -> egui::PaintCallbackInfo {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(128.0, 128.0));
            egui::PaintCallbackInfo {
                viewport: rect,
                clip_rect: rect,
                pixels_per_point: 1.0,
                screen_size_px: [128, 128],
            }
        }

        fn read(gl: &glow::Context) -> Vec<u8> {
            let mut pixels = vec![0; 128 * 128 * 4];
            unsafe {
                gl.read_pixels(
                    0,
                    0,
                    128,
                    128,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    glow::PixelPackData::Slice(Some(&mut pixels)),
                );
            }
            pixels
        }

        #[test]
        fn all_presets_compile_and_render_with_native_opengl_and_layer_compositing() {
            let context = Context::new();
            let gl = Arc::new(context.glow());
            let mut renderer = Renderer::new(gl.clone()).unwrap();
            let output = Target::new(&gl, 128, 128).unwrap();
            let mut history = AudioHistory::default();
            let mut quiet_history = AudioHistory::default();
            let magnitudes: Vec<_> = (0..BINS)
                .map(|i| 0.003 + (i as f32 * 0.13).sin().abs() * 0.03)
                .collect();
            let quiet_magnitudes: Vec<_> = (0..BINS)
                .map(|i| 0.00001 + (i as f32 * 0.13).sin().abs() * 0.0001)
                .collect();
            for _ in 0..HISTORY {
                history.push(&magnitudes, [-0.5, 0.5].into_iter());
                quiet_history.push(&quiet_magnitudes, [-0.01, 0.01].into_iter());
            }
            unsafe {
                gl.scissor(0, 0, 128, 128);
            }
            let mut errors = Vec::new();
            let mut blank = Vec::new();
            for (index, preset) in presets().iter().enumerate() {
                let draw = Draw {
                    preset: index,
                    channel: 0,
                    tint: [255; 3],
                    opacity: 255,
                    time: 12.0,
                    mouse: [0.0; 2],
                };
                let mut visible = false;
                // Some original artworks disappear at silence; others clip at saturated input.
                for (revision, audio) in [(1, &history), (2, &quiet_history)] {
                    match renderer.paint(info(), Some(output.framebuffer), &draw, audio, revision) {
                        Ok(()) => {
                            let pixels = read(&gl);
                            visible |= pixels
                                .as_chunks::<4>()
                                .0
                                .iter()
                                .any(|pixel| pixel[..3] != pixels[..3]);
                            let error = unsafe { gl.get_error() };
                            if error != glow::NO_ERROR {
                                errors.push(format!("{}: OpenGL error {error:#x}", preset.name));
                            }
                        }
                        Err(error) => errors.push(error),
                    }
                }
                if !visible {
                    blank.push(&preset.name);
                }
            }
            assert!(errors.is_empty(), "{}", errors.join("\n\n"));
            assert!(
                blank.is_empty(),
                "presets without visible geometry: {blank:?}"
            );

            let mut draw = Draw {
                preset: 0,
                channel: 0,
                tint: [255; 3],
                opacity: 255,
                time: 12.0,
                mouse: [0.0; 2],
            };
            renderer
                .paint(info(), Some(output.framebuffer), &draw, &history, 1)
                .unwrap();
            let opaque = read(&gl);
            unsafe {
                gl.clear_color(0.0, 0.0, 0.0, 1.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
            }
            draw.tint = [255, 0, 0];
            draw.opacity = 128;
            renderer
                .paint(info(), Some(output.framebuffer), &draw, &history, 1)
                .unwrap();
            let tinted = read(&gl);
            for (original, tinted) in opaque
                .as_chunks::<4>()
                .0
                .iter()
                .zip(tinted.as_chunks::<4>().0)
            {
                assert!((tinted[0] as f32 - original[0] as f32 * 128.0 / 255.0).abs() <= 2.0);
                assert_eq!(tinted[1], 0);
                assert_eq!(tinted[2], 0);
            }
            assert_eq!(unsafe { gl.get_error() }, glow::NO_ERROR);
            renderer.destroy();
            output.destroy(&gl);
        }
    }
}
