use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use rquickjs::{CaughtError, Context, Function, Runtime, Value};
use serde::{Deserialize, Serialize};

pub const DEFAULT_MEMORY_LIMIT_MB: u32 = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
pub enum Mode {
    #[default]
    Bytebeat,
    #[serde(alias = "Signed Bytebeat")]
    #[value(alias = "signed-bytebeat")]
    Signed,
    Floatbeat,
    Funcbeat,
}

impl Mode {
    pub const ALL: [Self; 4] = [
        Self::Bytebeat,
        Self::Signed,
        Self::Floatbeat,
        Self::Funcbeat,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Bytebeat => "Bytebeat",
            Self::Signed => "Signed Bytebeat",
            Self::Floatbeat => "Floatbeat",
            Self::Funcbeat => "Funcbeat",
        }
    }
}

pub struct Engine {
    // Keep the runtime alive until after its context is dropped.
    context: Context,
    _runtime: Runtime,
    deadline: Arc<Mutex<Instant>>,
    mode: Mode,
    rate: f64,
    sample: u64,
    phase: f64,
    held: [f32; 2],
    initialized: bool,
    thrown_texts: u64,
    warned_nan: bool,
}

impl Engine {
    pub fn new(code: &str, mode: Mode, rate: f64, memory_limit_mb: u32) -> Result<Self, String> {
        if !rate.is_finite() || !(1_000.0..=192_000.0).contains(&rate) {
            return Err("Sample rate must be between 1000 and 192000 Hz.".into());
        }
        let memory_limit = usize::try_from(memory_limit_mb)
            .ok()
            .filter(|limit| *limit > 0)
            .and_then(|limit| limit.checked_mul(1024 * 1024))
            .ok_or("Memory limit must be positive and fit this platform's address space.")?;
        let runtime = Runtime::new().map_err(|e| e.to_string())?;
        runtime.set_memory_limit(memory_limit);
        runtime.set_max_stack_size(512 * 1024);
        let deadline = Arc::new(Mutex::new(Instant::now() + Duration::from_millis(100)));
        let timeout = deadline.clone();
        runtime.set_interrupt_handler(Some(Box::new(move || {
            Instant::now() > *timeout.lock().expect("deadline mutex poisoned")
        })));
        let context = Context::full(&runtime).map_err(|e| e.to_string())?;
        context.with(|ctx| {
            ctx.eval::<(), _>(
                "for (const k of Object.getOwnPropertyNames(Math)) {
                    if (!(k in globalThis)) globalThis[k] = Math[k];
                }
                globalThis.int = Math.floor;
                globalThis.window = globalThis;
                globalThis.self = globalThis;",
            )
            .map_err(|e| js_error(&ctx, e))?;
            ctx.globals()
                .set("sampleRate", rate)
                .map_err(|e| js_error(&ctx, e))?;
            let body = if mode == Mode::Funcbeat {
                code.to_owned()
            } else {
                format!("return 0,\n{code}\n;")
            };
            // Pass source as an argument, never interpolate it into a JS string.
            let constructor: Function = ctx
                .globals()
                .get("Function")
                .map_err(|e| js_error(&ctx, e))?;
            let function: Function = if mode == Mode::Funcbeat {
                constructor.call((body,))
            } else {
                constructor.call(("t", "sampleRate", body))
            }
            .map_err(|e| js_error(&ctx, e))?;
            let function = if mode == Mode::Funcbeat {
                function
                    .call::<_, Function>(())
                    .map_err(|e| js_error(&ctx, e))?
            } else {
                function
            };
            ctx.globals()
                .set("__bytebeat", function)
                .map_err(|e| js_error(&ctx, e))
        })?;
        Ok(Self {
            context,
            _runtime: runtime,
            deadline,
            mode,
            rate,
            sample: 0,
            phase: 0.0,
            held: [0.0; 2],
            initialized: false,
            thrown_texts: 0,
            warned_nan: false,
        })
    }

    pub fn position(&self) -> u64 {
        self.sample.saturating_sub(1)
    }

    pub fn thrown_texts(&self) -> u64 {
        self.thrown_texts
    }

    pub fn render(&mut self, frames: usize, output_rate: u32) -> Result<Vec<[f32; 2]>, String> {
        if output_rate == 0 {
            return Err("Output sample rate must be positive.".into());
        }
        *self.deadline.lock().expect("deadline mutex poisoned") =
            Instant::now() + Duration::from_millis(100);
        let result: Result<Vec<[f32; 2]>, String> = self.context.with(|ctx| {
            let function: Function = ctx
                .globals()
                .get("__bytebeat")
                .map_err(|e| js_error(&ctx, e))?;
            let mut output = Vec::with_capacity(frames);
            for _ in 0..frames {
                while !self.initialized || self.phase >= f64::from(output_rate) {
                    if self.initialized {
                        self.phase -= f64::from(output_rate);
                    }
                    let t = if self.mode == Mode::Funcbeat {
                        self.sample as f64 / self.rate
                    } else {
                        self.sample as f64
                    };
                    let value: Option<Value> = match function.call((t, self.rate)) {
                        Ok(value) => Some(value),
                        Err(error) => {
                            let caught = CaughtError::from_error(&ctx, error);
                            if let CaughtError::Value(value) = &caught
                                && let Some(text) = value.as_string()
                            {
                                let text = text.to_string().map_err(|e| e.to_string())?;
                                log::warn!(
                                    "Thrown text at sample {} ({:.6} s):\n{text}",
                                    self.sample,
                                    self.sample as f64 / self.rate
                                );
                                self.thrown_texts += 1;
                                None
                            } else {
                                return Err(caught.to_string());
                            }
                        }
                    };
                    if let Some(value) = value {
                        let raw = if let Some(array) = value.as_array() {
                            if array.len() != 2 {
                                return Err("Stereo expressions must return [left, right].".into());
                            }
                            [
                                array.get::<f64>(0).map_err(|e| js_error(&ctx, e))?,
                                array.get::<f64>(1).map_err(|e| js_error(&ctx, e))?,
                            ]
                        } else {
                            let number = value
                                .as_number()
                                .ok_or("Expression must return a number or [left, right].")?;
                            [number; 2]
                        };
                        let mut next = self.held;
                        for (channel, value) in raw.into_iter().enumerate() {
                            if value.is_nan() {
                                if !self.warned_nan {
                                    log::warn!(
                                        "NaN output at sample {} ({:.6} s); holding the last valid \
                                         output per channel (initially silence). Further NaN \
                                         warnings are suppressed for this compilation.",
                                        self.sample,
                                        self.sample as f64 / self.rate
                                    );
                                    self.warned_nan = true;
                                }
                            } else {
                                next[channel] = normalize(value, self.mode)?;
                            }
                        }
                        self.held = next;
                    }
                    self.sample += 1;
                    self.initialized = true;
                }
                output.push(self.held);
                self.phase += self.rate;
            }
            Ok(output)
        });
        result.map_err(|error| {
            format!(
                "sample {} ({:.6} s): {error}",
                self.sample,
                self.sample as f64 / self.rate
            )
        })
    }
}

fn js_error(ctx: &rquickjs::Ctx<'_>, error: rquickjs::Error) -> String {
    let caught = CaughtError::from_error(ctx, error);
    if let CaughtError::Value(value) = &caught
        && let Some(text) = value.as_string()
    {
        return match text.to_string() {
            Ok(text) => format!("Thrown text: {text}"),
            Err(error) => format!("Could not read thrown text: {error}"),
        };
    }
    caught.to_string()
}

fn normalize(value: f64, mode: Mode) -> Result<f32, String> {
    if !value.is_finite() {
        return Err("Expression produced NaN or infinity; playback was paused.".into());
    }
    Ok(match mode {
        Mode::Bytebeat => (value.trunc().rem_euclid(256.0) / 128.0 - 1.0) as f32,
        Mode::Signed => ((value.trunc() + 128.0).rem_euclid(256.0) / 128.0 - 1.0) as f32,
        Mode::Floatbeat | Mode::Funcbeat => value.clamp(-1.0, 1.0) as f32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapping_and_signed_samples() {
        assert_eq!(normalize(256.0, Mode::Bytebeat).unwrap(), -1.0);
        assert_eq!(normalize(-1.0, Mode::Bytebeat).unwrap(), 127.0 / 128.0);
        assert_eq!(normalize(128.0, Mode::Signed).unwrap(), -1.0);
        assert_eq!(normalize(-128.0, Mode::Signed).unwrap(), -1.0);
        assert_eq!(normalize(-1.0, Mode::Signed).unwrap(), -1.0 / 128.0);
        assert_eq!(normalize(0.0, Mode::Signed).unwrap(), 0.0);
    }

    #[test]
    fn javascript_stereo_and_resampling() {
        let mut engine = Engine::new("[t, 255-t]", Mode::Bytebeat, 8_000.0, 32).unwrap();
        let frames = engine.render(4, 16_000).unwrap();
        assert_eq!(frames[0], [-1.0, 127.0 / 128.0]);
        assert_eq!(frames[0], frames[1]);
        assert_eq!(frames[2], [-127.0 / 128.0, 126.0 / 128.0]);
        assert_eq!(frames[2], frames[3]);
    }

    #[test]
    fn math_aliases_and_function_state() {
        let mut engine = Engine::new(
            "let n = 0; return t => [sin(PI/2), n++ / 10];",
            Mode::Funcbeat,
            8_000.0,
            32,
        )
        .unwrap();
        assert_eq!(
            engine.render(2, 8_000).unwrap(),
            vec![[1.0, 0.0], [1.0, 0.1]]
        );
    }

    #[test]
    fn rejects_bad_programs_and_infinite_output() {
        assert!(Engine::new("t +", Mode::Bytebeat, 8_000.0, 32).is_err());
        assert!(Engine::new("return 42", Mode::Funcbeat, 8_000.0, 32).is_err());
        assert!(Engine::new("t", Mode::Bytebeat, 0.0, 32).is_err());
        for code in [
            "Infinity",
            "-Infinity",
            "[NaN, Infinity]",
            "[-Infinity, NaN]",
            "[1,2,3]",
            "'hello'",
            "(()=>{throw Error('oops')})()",
        ] {
            let mut engine = Engine::new(code, Mode::Bytebeat, 8_000.0, 32).unwrap();
            assert!(engine.render(1, 8_000).is_err(), "{code}");
        }
    }

    #[test]
    fn nan_starts_with_silence_and_holds_normalized_output_in_all_modes() {
        for mode in Mode::ALL {
            let code = if mode == Mode::Funcbeat {
                "let n = 0; return () => [NaN, 192, NaN, NaN, 64][n++];"
            } else {
                "[NaN, 192, NaN, NaN, 64][t]"
            };
            let mut engine = Engine::new(code, mode, 8_000.0, 32).unwrap();
            let first = normalize(192.0, mode).unwrap();
            let last = normalize(64.0, mode).unwrap();
            assert_eq!(
                engine.render(3, 8_000).unwrap(),
                vec![[0.0; 2], [first; 2], [first; 2]],
                "{mode:?}"
            );
            assert!(engine.warned_nan);
            assert_eq!(
                engine.render(2, 8_000).unwrap(),
                vec![[first; 2], [last; 2]],
                "{mode:?}"
            );
            assert_eq!(engine.position(), 4);
            assert_eq!(engine.thrown_texts(), 0);
        }
    }

    #[test]
    fn nan_recovery_is_per_channel_and_preserves_resampling() {
        let mut engine = Engine::new(
            "[[NaN, 0.25], [0.5, NaN], [NaN, -0.5], [-0.25, 0.75]][t]",
            Mode::Floatbeat,
            8_000.0,
            32,
        )
        .unwrap();
        assert_eq!(
            engine.render(8, 16_000).unwrap(),
            vec![
                [0.0, 0.25],
                [0.0, 0.25],
                [0.5, 0.25],
                [0.5, 0.25],
                [0.5, -0.5],
                [0.5, -0.5],
                [-0.25, 0.75],
                [-0.25, 0.75],
            ]
        );
        assert_eq!(engine.position(), 3);
    }

    #[test]
    fn persistent_nan_remains_silent() {
        let mut engine = Engine::new("NaN", Mode::Bytebeat, 8_000.0, 32).unwrap();
        assert_eq!(engine.render(4, 8_000).unwrap(), vec![[0.0; 2]; 4]);
        assert_eq!(engine.render(4, 8_000).unwrap(), vec![[0.0; 2]; 4]);
        assert!(engine.warned_nan);
        assert_eq!(engine.position(), 7);
    }

    #[test]
    fn interrupts_infinite_loops() {
        let mut engine =
            Engine::new("(()=>{while(true){} })()", Mode::Bytebeat, 8_000.0, 32).unwrap();
        let start = Instant::now();
        assert!(engine.render(1, 8_000).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn funcbeat_uses_seconds() {
        let mut engine = Engine::new("return t => t;", Mode::Funcbeat, 1_000.0, 32).unwrap();
        let frames = engine.render(3, 1_000).unwrap();
        assert_eq!(frames, vec![[0.0; 2], [0.001; 2], [0.002; 2]]);
    }

    #[test]
    fn resampling_is_exact_across_block_boundaries() {
        let mut engine = Engine::new("t & 255", Mode::Bytebeat, 8_000.0, 32).unwrap();
        let frames: Vec<_> = (0..375)
            .flat_map(|_| engine.render(128, 48_000).unwrap())
            .collect();
        assert_eq!(frames.len(), 48_000);
        for (i, frame) in frames.iter().enumerate() {
            let expected = normalize((i / 6) as f64, Mode::Bytebeat).unwrap();
            assert_eq!(*frame, [expected; 2], "output sample {i}");
        }
        assert_eq!(engine.position(), 7_999);
        assert!(engine.render(1, 0).is_err());
    }

    #[test]
    fn downsampling_still_evaluates_every_source_sample() {
        let mut engine = Engine::new(
            "return () => (globalThis.n = (globalThis.n || 0) + 0.01);",
            Mode::Funcbeat,
            48_000.0,
            32,
        )
        .unwrap();
        let frames = engine.render(3, 12_000).unwrap();
        assert!((frames[0][0] - 0.01).abs() < 1e-6);
        assert!((frames[1][0] - 0.05).abs() < 1e-6);
        assert!((frames[2][0] - 0.09).abs() < 1e-6);
    }

    #[test]
    fn errors_include_the_javascript_message() {
        let mut engine = Engine::new(
            "(()=>{throw Error('test diagnostic')})()",
            Mode::Bytebeat,
            8_000.0,
            32,
        )
        .unwrap();
        let error = engine.render(1, 8_000).unwrap_err();
        assert!(error.contains("test diagnostic"), "{error}");
    }

    #[test]
    fn interrupts_funcbeat_initialization() {
        let start = Instant::now();
        assert!(Engine::new("while (true) {}", Mode::Funcbeat, 8_000.0, 32).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn accepts_trailing_semicolons_and_fractional_rates() {
        let mut engine = Engine::new(
            "sin(t); // upstream-style expression",
            Mode::Floatbeat,
            35_498.666_66,
            32,
        )
        .unwrap();
        assert_eq!(engine.render(1, 48_000).unwrap(), vec![[0.0; 2]]);
        assert!(engine.render(128, 48_000).is_ok());
    }

    #[test]
    fn funcbeat_factory_can_declare_its_own_time_and_rate() {
        let mut engine = Engine::new(
            "let t = 0, sampleRate = 16000; return seconds => ++t / sampleRate;",
            Mode::Funcbeat,
            48_000.0,
            32,
        )
        .unwrap();
        let frames = engine.render(2, 48_000).unwrap();
        assert_eq!(frames, vec![[1.0 / 16000.0; 2], [2.0 / 16000.0; 2]]);
    }

    #[test]
    fn global_namespace_aliases_do_not_require_a_browser() {
        let mut engine = Engine::new(
            "window === globalThis && self === globalThis && window.sampleRate === 8000 ? 0.25 : 0",
            Mode::Floatbeat,
            8_000.0,
            32,
        )
        .unwrap();
        assert_eq!(engine.render(1, 48_000).unwrap(), vec![[0.25; 2]]);
    }

    #[test]
    fn thrown_text_starts_with_silence_then_holds_the_previous_sample() {
        let mut engine = Engine::new(
            "t === 0 || t === 2 ? (() => { throw 'display text'; })() : 192",
            Mode::Bytebeat,
            8_000.0,
            32,
        )
        .unwrap();
        assert_eq!(
            engine.render(4, 8_000).unwrap(),
            vec![[0.0; 2], [0.5; 2], [0.5; 2], [0.5; 2]]
        );
        assert_eq!(engine.thrown_texts(), 2);
        assert_eq!(engine.position(), 3);
        assert_eq!(engine.render(1, 8_000).unwrap(), vec![[0.5; 2]]);
    }

    #[test]
    fn thrown_text_preserves_both_stereo_channels() {
        let mut engine = Engine::new(
            "t === 1 ? (() => { throw 'stereo'; })() : [t / 4, -0.5]",
            Mode::Floatbeat,
            8_000.0,
            32,
        )
        .unwrap();
        assert_eq!(
            engine.render(3, 8_000).unwrap(),
            vec![[0.0, -0.5], [0.0, -0.5], [0.5, -0.5]]
        );
    }

    #[test]
    fn funcbeat_thrown_text_advances_time_and_keeps_closure_state() {
        let mut engine = Engine::new(
            "let n = 0; return seconds => { if (++n === 1) throw 'start'; return seconds; };",
            Mode::Funcbeat,
            1_000.0,
            32,
        )
        .unwrap();
        assert_eq!(
            engine.render(3, 1_000).unwrap(),
            vec![[0.0; 2], [0.001; 2], [0.002; 2]]
        );
        assert_eq!(engine.thrown_texts(), 1);
    }

    #[test]
    fn only_text_thrown_during_sample_evaluation_is_recoverable() {
        for code in [
            "(() => { throw Error('fatal'); })()",
            "(() => { throw null; })()",
            "(() => { throw 42; })()",
            "(() => { throw {message: 'not text'}; })()",
            "t === 0 ? (() => { throw 'first'; })() : Infinity",
        ] {
            let mut engine = Engine::new(code, Mode::Bytebeat, 8_000.0, 32).unwrap();
            assert!(engine.render(2, 8_000).is_err(), "{code}");
        }
        let error = Engine::new("throw 'factory diagnostic';", Mode::Funcbeat, 8_000.0, 32)
            .err()
            .expect("factory must still return a function");
        assert!(error.contains("Thrown text: factory diagnostic"), "{error}");
    }

    #[test]
    fn memory_limit_controls_actual_javascript_allocations() {
        let code = "(t || (globalThis.buffer = new Uint8Array(2 * 1024 * 1024)), 128)";
        let mut small = Engine::new(code, Mode::Bytebeat, 8_000.0, 1).unwrap();
        let error = small.render(1, 8_000).unwrap_err();
        assert!(error.contains("out of memory"), "{error}");
        let mut large = Engine::new(code, Mode::Bytebeat, 8_000.0, 8).unwrap();
        assert_eq!(large.render(1, 8_000).unwrap(), vec![[0.0; 2]]);
        assert!(Engine::new("t", Mode::Bytebeat, 8_000.0, 0).is_err());
    }
}
