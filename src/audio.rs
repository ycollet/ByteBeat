use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rtrb::{Consumer, RingBuffer};

use crate::engine::{Engine, Mode};

const BUFFER: usize = 2048;
pub const BLOCK: usize = 128;

pub enum Command {
    Load {
        code: String,
        mode: Mode,
        rate: f64,
        play: bool,
    },
    Play(bool),
    Shutdown,
}

#[derive(Default)]
pub struct Snapshot {
    pub frames: Vec<[f32; 2]>,
    pub generation: u64,
    pub position: u64,
    pub output_rate: u32,
    pub playing: bool,
    pub error: Option<String>,
}

struct Shared {
    playing: AtomicBool,
    generation: AtomicU64,
    volume: AtomicU32,
    dc_blocker: AtomicBool,
    underruns: AtomicU64,
}

pub struct Audio {
    sender: Sender<Command>,
    pub snapshot: Arc<Mutex<Snapshot>>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Audio {
    pub fn new(
        memory_limit_mb: u32,
        use_jack: bool,
        client_id: Option<String>,
    ) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel();
        let (ready, startup) = mpsc::channel();
        let managed = client_id.is_some();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let shared = Arc::new(Shared {
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            volume: AtomicU32::new(0.25_f32.to_bits()),
            dc_blocker: AtomicBool::new(true),
            underruns: AtomicU64::new(0),
        });
        let state = snapshot.clone();
        let controls = shared.clone();
        let worker = thread::spawn(move || {
            if let Err(error) = run(
                receiver,
                state.clone(),
                controls,
                memory_limit_mb,
                use_jack,
                client_id,
                &ready,
            ) {
                let _ = ready.send(Err(error.clone()));
                log::error!("{error}");
                let mut state = state.lock().expect("audio snapshot mutex poisoned");
                state.playing = false;
                state.error = Some(error);
            }
        });
        let audio = Self {
            sender,
            snapshot,
            shared,
            worker: Some(worker),
        };
        if managed {
            startup
                .recv()
                .map_err(|e| format!("Audio startup: {e}"))??;
        }
        Ok(audio)
    }

    pub fn send(&self, command: Command) {
        if self.sender.send(command).is_err() {
            log::error!("Audio worker is unavailable");
            self.snapshot
                .lock()
                .expect("audio snapshot mutex poisoned")
                .error = Some(
                "Audio worker is unavailable. Check your output device and restart the app.".into(),
            );
        }
    }

    pub fn volume(&self, volume: f32) {
        self.shared
            .volume
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn underruns(&self) -> u64 {
        self.shared.underruns.load(Ordering::Relaxed)
    }

    pub fn dc_blocker(&self, enabled: bool) {
        self.shared.dc_blocker.store(enabled, Ordering::Relaxed);
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        let _ = self.sender.send(Command::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

type Frame = ([f32; 2], u64);

struct DcBlocker {
    pole: f64,
    input: [f64; 2],
    output: [f64; 2],
    generation: u64,
    enabled: bool,
}

impl DcBlocker {
    fn new(output_rate: u32) -> Self {
        Self {
            pole: (-std::f64::consts::TAU * 10.0 / f64::from(output_rate)).exp(),
            input: [0.0; 2],
            output: [0.0; 2],
            generation: 0,
            enabled: false,
        }
    }

    fn reset(&mut self) {
        self.input = [0.0; 2];
        self.output = [0.0; 2];
    }

    fn process(&mut self, frame: [f32; 2]) -> [f32; 2] {
        std::array::from_fn(|channel| {
            let input = f64::from(frame[channel]);
            // Normalize the one-pole high-pass to unity gain at Nyquist.
            let output = (1.0 + self.pole) * 0.5 * (input - self.input[channel])
                + self.pole * self.output[channel];
            self.input[channel] = input;
            self.output[channel] = output;
            output as f32
        })
    }
}

fn output_host(use_jack: bool) -> Result<cpal::Host, String> {
    if !use_jack {
        return Ok(cpal::default_host());
    }
    #[cfg(target_os = "linux")]
    {
        cpal::host_from_id(cpal::HostId::Jack).map_err(|error| {
            format!("JACK backend unavailable: {error}. Check your JACK installation.")
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err("--jack is only supported on Linux.".into())
    }
}

fn output_device(use_jack: bool, client_id: Option<&str>) -> Result<cpal::Device, String> {
    #[cfg(target_os = "linux")]
    if use_jack && let Some(id) = client_id {
        return cpal::platform::JackDevice::default_output_device(
            &format!("{id}/audio"),
            false,
            false,
        )
        .map(Into::into)
        .map_err(|e| format!("NSM JACK output: {e}"));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = client_id;
    let host = output_host(use_jack)?;
    log::info!("Audio backend: {}", host.id().name());
    host.default_output_device().ok_or_else(|| {
        if use_jack {
            "No JACK output device available. Install libjack, start a JACK server (or enable PipeWire's JACK support), then restart the app.".to_owned()
        } else {
            "No audio output device found. Connect an output device and restart the app.".to_owned()
        }
    })
}

fn run(
    receiver: Receiver<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    shared: Arc<Shared>,
    memory_limit_mb: u32,
    use_jack: bool,
    client_id: Option<String>,
    ready: &Sender<Result<(), String>>,
) -> Result<(), String> {
    let device = output_device(use_jack, client_id.as_deref())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("Audio configuration: {e}"))?;
    let rate = config.sample_rate().0;
    let (mut producer, consumer) = RingBuffer::<Frame>::new(BUFFER);
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build_stream::<f32>(
            &device,
            &config.into(),
            consumer,
            shared.clone(),
            snapshot.clone(),
        ),
        cpal::SampleFormat::I16 => build_stream::<i16>(
            &device,
            &config.into(),
            consumer,
            shared.clone(),
            snapshot.clone(),
        ),
        cpal::SampleFormat::U16 => build_stream::<u16>(
            &device,
            &config.into(),
            consumer,
            shared.clone(),
            snapshot.clone(),
        ),
        format => return Err(format!("Unsupported output sample format: {format:?}")),
    }?;
    stream
        .play()
        .map_err(|e| format!("Starting audio output: {e}"))?;
    snapshot
        .lock()
        .expect("audio snapshot mutex poisoned")
        .output_rate = rate;
    let _ = ready.send(Ok(()));
    let mut engine: Option<Engine> = None;
    let mut history = VecDeque::with_capacity(BUFFER);
    loop {
        while let Ok(command) = receiver.try_recv() {
            match command {
                Command::Shutdown => return Ok(()),
                Command::Play(play) => {
                    shared
                        .playing
                        .store(play && engine.is_some(), Ordering::Relaxed);
                    snapshot
                        .lock()
                        .expect("audio snapshot mutex poisoned")
                        .playing = play && engine.is_some();
                }
                Command::Load {
                    code,
                    mode,
                    rate,
                    play,
                } => {
                    log::info!(
                        "Loading {} at {rate} Hz ({} bytes)",
                        mode.label(),
                        code.len()
                    );
                    shared.playing.store(false, Ordering::Relaxed);
                    let generation = shared
                        .generation
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1);
                    history.clear();
                    let result = Engine::new(&code, mode, rate, memory_limit_mb);
                    let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                    state.generation = generation;
                    state.position = 0;
                    state.frames.clear();
                    match result {
                        Ok(new_engine) => {
                            engine = Some(new_engine);
                            state.error = None;
                            state.playing = play;
                            shared.playing.store(play, Ordering::Relaxed);
                        }
                        Err(error) => {
                            log::error!("Compilation failed: {error}");
                            engine = None;
                            state.error = Some(error);
                            state.playing = false;
                        }
                    }
                }
            }
        }
        if shared.playing.load(Ordering::Relaxed) && producer.slots() >= BLOCK {
            if let Some(engine) = engine.as_mut() {
                match engine.render(BLOCK, rate) {
                    Ok(frames) => {
                        let generation = shared.generation.load(Ordering::Relaxed);
                        for frame in frames {
                            // Only this thread produces, and capacity was checked above.
                            producer
                                .push((frame, generation))
                                .map_err(|e| format!("Audio queue: {e}"))?;
                            history.push_back(frame);
                            if history.len() > BUFFER {
                                history.pop_front();
                            }
                        }
                        let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                        state.position = engine.position();
                        state.frames = history.iter().copied().collect();
                    }
                    Err(error) => {
                        log::error!("Rendering failed: {error}");
                        shared.playing.store(false, Ordering::Relaxed);
                        let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                        state.playing = false;
                        state.error = Some(error);
                    }
                }
            }
        } else {
            thread::sleep(Duration::from_millis(2));
        }
    }
}

fn build_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut consumer: Consumer<Frame>,
    shared: Arc<Shared>,
    snapshot: Arc<Mutex<Snapshot>>,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels as usize;
    let callback_shared = shared.clone();
    let mut dc_blocker = DcBlocker::new(config.sample_rate.0);
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                fill_output(
                    output,
                    channels,
                    &mut consumer,
                    &callback_shared,
                    &mut dc_blocker,
                );
            },
            move |error| {
                log::error!("Audio output failed: {error}");
                shared.playing.store(false, Ordering::Relaxed);
                let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                state.playing = false;
                state.error = Some(format!("Audio output failed: {error}"));
            },
            None,
        )
        .map_err(|e| format!("Opening audio output: {e}"))
}

fn fill_output<T>(
    output: &mut [T],
    channels: usize,
    consumer: &mut Consumer<Frame>,
    shared: &Shared,
    dc_blocker: &mut DcBlocker,
) where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let playing = shared.playing.load(Ordering::Relaxed);
    let generation = shared.generation.load(Ordering::Relaxed);
    let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
    let enabled = shared.dc_blocker.load(Ordering::Relaxed);
    if dc_blocker.enabled != enabled || dc_blocker.generation != generation || !playing {
        dc_blocker.reset();
    }
    dc_blocker.enabled = enabled;
    dc_blocker.generation = generation;
    for output_frame in output.chunks_mut(channels) {
        let mut frame = [0.0; 2];
        if playing {
            loop {
                match consumer.pop() {
                    Ok((samples, id)) if id == generation => {
                        frame = samples;
                        if enabled {
                            frame = dc_blocker.process(frame);
                        }
                        break;
                    }
                    Ok(_) => continue,
                    Err(_) => {
                        shared.underruns.fetch_add(1, Ordering::Relaxed);
                        dc_blocker.reset();
                        break;
                    }
                }
            }
        }
        for (channel, sample) in output_frame.iter_mut().enumerate() {
            let value = if channels == 1 {
                (frame[0] + frame[1]) * 0.5
            } else if channel < 2 {
                frame[channel]
            } else {
                0.0
            };
            let value = value * volume;
            // DC removal can briefly overshoot on steps; keep device conversion in range.
            *sample = T::from_sample(if enabled {
                value.clamp(-1.0, 1.0)
            } else {
                value
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared {
            playing: AtomicBool::new(true),
            generation: AtomicU64::new(1),
            volume: AtomicU32::new(0.5_f32.to_bits()),
            dc_blocker: AtomicBool::new(false),
            underruns: AtomicU64::new(0),
        }
    }

    #[test]
    fn dc_blocker_rejects_dc_and_preserves_audio_at_device_rates() {
        for rate in [8_000, 44_100, 48_000, 192_000] {
            let mut filter = DcBlocker::new(rate);
            for _ in 0..rate / 4 {
                filter.process([0.75, -0.25]);
            }
            let output = filter.process([0.75, -0.25]);
            assert!(output.iter().all(|sample| sample.abs() < 1e-6));

            for (frequency, expected_gain) in [(10.0, 1.0 / 2.0_f64.sqrt()), (1000.0, 1.0)] {
                filter.reset();
                let mut energy = 0.0;
                for index in 0..rate {
                    let wave = (std::f64::consts::TAU * frequency * f64::from(index)
                        / f64::from(rate))
                    .sin() as f32
                        * 0.4;
                    let output = filter.process([wave + 0.25, 0.0]);
                    assert_eq!(output[1], 0.0);
                    if index >= rate / 2 {
                        energy += f64::from(output[0]).powi(2);
                    }
                }
                let gain = (energy / f64::from(rate / 2)).sqrt() / (0.4 / 2.0_f64.sqrt());
                assert!(
                    (gain - expected_gain).abs() < 0.002,
                    "{rate} Hz, {frequency} Hz: {gain}"
                );
            }
        }
    }

    #[test]
    fn dc_blocker_retains_state_between_callbacks_and_resets_on_discontinuities() {
        let shared = shared();
        shared.dc_blocker.store(true, Ordering::Relaxed);
        let mut filter = DcBlocker::new(48_000);
        let (mut producer, mut consumer) = RingBuffer::new(8);
        let mut output = [0.0_f32; 2];
        producer.push(([0.5, -0.25], 1)).unwrap();
        fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
        let first = output;
        producer.push(([0.5, -0.25], 1)).unwrap();
        fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
        assert!(output[0] < first[0]);
        assert!(output[1].abs() < first[1].abs());

        for reset in ["pause", "underrun", "generation", "toggle"] {
            match reset {
                "pause" => {
                    shared.playing.store(false, Ordering::Relaxed);
                    fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
                    assert_eq!(output, [0.0; 2]);
                    shared.playing.store(true, Ordering::Relaxed);
                }
                "underrun" => {
                    fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
                    assert_eq!(output, [0.0; 2]);
                }
                "generation" => {
                    shared.generation.fetch_add(1, Ordering::Relaxed);
                }
                "toggle" => {
                    shared.dc_blocker.store(false, Ordering::Relaxed);
                    producer
                        .push(([0.5, -0.25], shared.generation.load(Ordering::Relaxed)))
                        .unwrap();
                    fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
                    assert_eq!(output, [0.25, -0.125]);
                    shared.dc_blocker.store(true, Ordering::Relaxed);
                }
                _ => unreachable!(),
            }
            producer
                .push(([0.5, -0.25], shared.generation.load(Ordering::Relaxed)))
                .unwrap();
            fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
            assert_eq!(output, first, "{reset}");
        }
    }

    #[test]
    fn dc_blocker_bounds_transients_for_float_and_integer_devices() {
        let shared = shared();
        shared.dc_blocker.store(true, Ordering::Relaxed);
        shared.volume.store(1.0_f32.to_bits(), Ordering::Relaxed);
        let mut filter = DcBlocker::new(48_000);
        let (mut producer, mut consumer) = RingBuffer::new(8);
        filter.input = [1.0, -1.0];
        filter.enabled = true;
        filter.generation = 1;
        producer.push(([-1.0, 1.0], 1)).unwrap();
        let mut float = [0.0_f32; 2];
        fill_output(&mut float, 2, &mut consumer, &shared, &mut filter);
        assert_eq!(float, [-1.0, 1.0]);
        filter.input = [1.0, -1.0];
        filter.output = [0.0; 2];
        producer.push(([-1.0, 1.0], 1)).unwrap();
        let mut signed = [0_i16; 2];
        fill_output(&mut signed, 2, &mut consumer, &shared, &mut filter);
        assert_eq!(signed, [i16::MIN, i16::MAX]);
        filter.input = [1.0, -1.0];
        filter.output = [0.0; 2];
        producer.push(([-1.0, 1.0], 1)).unwrap();
        let mut unsigned = [0_u16; 2];
        fill_output(&mut unsigned, 2, &mut consumer, &shared, &mut filter);
        assert_eq!(unsigned, [u16::MIN, u16::MAX]);
    }

    #[test]
    fn pause_preserves_queued_audio_and_resume_plays_it() {
        let shared = shared();
        let mut filter = DcBlocker::new(48_000);
        let (mut producer, mut consumer) = RingBuffer::new(8);
        producer.push(([0.8, -0.4], 1)).unwrap();
        shared.playing.store(false, Ordering::Relaxed);
        let mut output = [1.0_f32; 2];
        fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
        assert_eq!(output, [0.0, 0.0]);
        assert_eq!(consumer.slots(), 1);
        shared.playing.store(true, Ordering::Relaxed);
        fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
        assert_eq!(output, [0.4, -0.2]);
    }

    #[test]
    fn restart_discards_old_generations_and_underruns_are_silent() {
        let shared = shared();
        let mut filter = DcBlocker::new(48_000);
        let (mut producer, mut consumer) = RingBuffer::new(8);
        producer.push(([1.0, 1.0], 0)).unwrap();
        producer.push(([0.2, -0.2], 1)).unwrap();
        let mut output = [1.0_f32; 4];
        fill_output(&mut output, 2, &mut consumer, &shared, &mut filter);
        assert_eq!(output, [0.1, -0.1, 0.0, 0.0]);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn mono_downmix_and_surround_channel_silence() {
        let shared = shared();
        let mut filter = DcBlocker::new(48_000);
        let (mut producer, mut consumer) = RingBuffer::new(8);
        producer.push(([0.8, 0.4], 1)).unwrap();
        let mut mono = [0.0_f32; 1];
        fill_output(&mut mono, 1, &mut consumer, &shared, &mut filter);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        producer.push(([0.8, 0.4], 1)).unwrap();
        let mut surround = [1.0_f32; 6];
        fill_output(&mut surround, 6, &mut consumer, &shared, &mut filter);
        assert_eq!(surround, [0.4, 0.2, 0.0, 0.0, 0.0, 0.0]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn jack_backend_is_compiled_in_on_linux() {
        assert!(cpal::ALL_HOSTS.contains(&cpal::HostId::Jack));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn requesting_jack_on_other_platforms_is_an_error() {
        let error = output_host(true)
            .err()
            .expect("JACK must not silently fall back");
        assert!(error.contains("only supported on Linux"), "{error}");
    }
}
