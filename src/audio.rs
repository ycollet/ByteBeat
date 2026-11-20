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
use rtrb::{Consumer, Producer, RingBuffer};
use serde::{Deserialize, Serialize};

use crate::engine::{Engine, Mode};

const HISTORY: usize = 2048;
const MAX_BUFFER: usize = 16384;
pub const BLOCK: usize = 128;
pub const MAX_PRE_RENDER_SECONDS: u32 = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BufferSize {
    #[default]
    Frames2048,
    Frames4096,
    Frames8192,
    Frames16384,
}

impl BufferSize {
    pub const ALL: [Self; 4] = [
        Self::Frames2048,
        Self::Frames4096,
        Self::Frames8192,
        Self::Frames16384,
    ];

    pub fn frames(self) -> usize {
        match self {
            Self::Frames2048 => 2048,
            Self::Frames4096 => 4096,
            Self::Frames8192 => 8192,
            Self::Frames16384 => 16384,
        }
    }
}

pub enum Command {
    Load {
        code: String,
        mode: Mode,
        rate: f64,
        play: bool,
        pre_render: Option<u32>,
    },
    Play(bool),
    CancelPreRender,
    Shutdown,
}

#[derive(Clone, Copy)]
pub struct PreRenderProgress {
    pub rendered_frames: usize,
    pub total_frames: usize,
}

#[derive(Default)]
pub struct Snapshot {
    pub frames: Vec<[f32; 2]>,
    pub generation: u64,
    pub position: u64,
    pub output_rate: u32,
    pub playing: bool,
    pub error: Option<String>,
    pub pre_render: Option<PreRenderProgress>,
    pub cached_frames: usize,
}

struct Shared {
    playing: AtomicBool,
    generation: AtomicU64,
    volume: AtomicU32,
    dc_blocker: AtomicBool,
    underruns: AtomicU64,
    buffer_frames: AtomicU32,
    output_error: Mutex<Option<String>>,
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
        buffer: BufferSize,
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
            buffer_frames: AtomicU32::new(buffer.frames() as u32),
            output_error: Mutex::new(None),
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
                state.pre_render = None;
                state.cached_frames = 0;
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

    pub fn buffer_size(&self, buffer: BufferSize) {
        self.shared
            .buffer_frames
            .store(buffer.frames() as u32, Ordering::Relaxed);
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

struct RenderBlock {
    frames: Vec<[f32; 2]>,
    position: u64,
}

struct PreparedAudio {
    frames: Vec<[f32; 2]>,
    blocks: VecDeque<(usize, u64)>,
    consumed: usize,
    target: usize,
    preparing: bool,
}

impl PreparedAudio {
    fn new(seconds: u32, output_rate: u32) -> Result<Self, String> {
        if !(1..=MAX_PRE_RENDER_SECONDS).contains(&seconds) || output_rate == 0 {
            return Err(format!(
                "Pre-render duration must be 1 to {MAX_PRE_RENDER_SECONDS} seconds with a valid output rate."
            ));
        }
        let target = usize::try_from(output_rate)
            .ok()
            .and_then(|rate| rate.checked_mul(seconds as usize))
            .ok_or("Pre-render size exceeds this platform's capacity.")?;
        let mut frames = Vec::new();
        frames
            .try_reserve_exact(target)
            .map_err(|e| format!("Allocating pre-render audio: {e}"))?;
        let mut blocks = VecDeque::new();
        blocks
            .try_reserve_exact(target.div_ceil(BLOCK))
            .map_err(|e| format!("Allocating pre-render positions: {e}"))?;
        Ok(Self {
            frames,
            blocks,
            consumed: 0,
            target,
            preparing: true,
        })
    }

    fn render(&mut self, engine: &mut Engine, output_rate: u32) -> Result<(), String> {
        let count = BLOCK.min(self.target - self.frames.len());
        self.frames.extend(engine.render(count, output_rate)?);
        self.blocks
            .push_back((self.frames.len(), engine.position()));
        self.preparing = self.frames.len() < self.target;
        Ok(())
    }

    fn progress(&self) -> PreRenderProgress {
        PreRenderProgress {
            rendered_frames: self.frames.len(),
            total_frames: self.target,
        }
    }

    fn remaining(&self) -> usize {
        self.frames.len() - self.consumed
    }

    fn next_len(&self) -> usize {
        self.blocks
            .front()
            .map_or(BLOCK, |(end, _)| end - self.consumed)
    }

    fn pop(&mut self) -> Option<RenderBlock> {
        let (end, position) = self.blocks.pop_front()?;
        let frames = self.frames[self.consumed..end].to_vec();
        self.consumed = end;
        Some(RenderBlock { frames, position })
    }
}

struct Playback {
    engine: Engine,
    prepared: Option<PreparedAudio>,
}

impl Playback {
    fn preparing(&self) -> bool {
        self.prepared.as_ref().is_some_and(|audio| audio.preparing)
    }

    fn prepare_block(&mut self, output_rate: u32) -> Result<(), String> {
        if let Some(audio) = &mut self.prepared {
            audio.render(&mut self.engine, output_rate)?;
        }
        Ok(())
    }

    fn cancel_pre_render(&mut self) {
        if let Some(audio) = &mut self.prepared {
            audio.preparing = false;
            audio.target = audio.frames.len();
        }
    }

    fn next_len(&self) -> usize {
        self.prepared
            .as_ref()
            .map_or(BLOCK, PreparedAudio::next_len)
    }

    fn next_block(&mut self, output_rate: u32) -> Result<RenderBlock, String> {
        if let Some(audio) = &mut self.prepared
            && let Some(block) = audio.pop()
        {
            if audio.remaining() == 0 {
                self.prepared = None;
            }
            return Ok(block);
        }
        self.prepared = None;
        Ok(RenderBlock {
            frames: self.engine.render(BLOCK, output_rate)?,
            position: self.engine.position(),
        })
    }

    fn update_pre_render(&self, snapshot: &mut Snapshot) {
        snapshot.pre_render = self.prepared.as_ref().map(PreparedAudio::progress);
        snapshot.cached_frames = self.prepared.as_ref().map_or(0, PreparedAudio::remaining);
    }
}

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
    let (producer, consumer) = RingBuffer::<Frame>::new(MAX_BUFFER);
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
    run_renderer(receiver, snapshot, shared, producer, rate, memory_limit_mb)
}

fn run_renderer(
    receiver: Receiver<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    shared: Arc<Shared>,
    mut producer: Producer<Frame>,
    output_rate: u32,
    memory_limit_mb: u32,
) -> Result<(), String> {
    let mut playback: Option<Playback> = None;
    let mut history = VecDeque::with_capacity(HISTORY);
    let mut requested_play = false;
    let mut priming = true;
    let mut prefilled = 0;
    loop {
        if let Some(error) = shared
            .output_error
            .lock()
            .expect("audio output error mutex poisoned")
            .clone()
        {
            return Err(error);
        }
        while let Ok(command) = receiver.try_recv() {
            match command {
                Command::Shutdown => return Ok(()),
                Command::Play(play) => {
                    requested_play = play && playback.is_some();
                    if let Some(playback) = &mut playback
                        && play
                    {
                        playback.cancel_pre_render();
                    }
                    shared
                        .playing
                        .store(requested_play && !priming, Ordering::Relaxed);
                    snapshot
                        .lock()
                        .expect("audio snapshot mutex poisoned")
                        .playing = requested_play;
                }
                Command::CancelPreRender => {
                    if let Some(playback) = &mut playback {
                        playback.cancel_pre_render();
                    }
                    snapshot
                        .lock()
                        .expect("audio snapshot mutex poisoned")
                        .pre_render = None;
                }
                Command::Load {
                    code,
                    mode,
                    rate,
                    play,
                    pre_render,
                } => {
                    log::info!(
                        "Loading {} at {rate} Hz ({} bytes)",
                        mode.label(),
                        code.len()
                    );
                    shared.playing.store(false, Ordering::Relaxed);
                    requested_play = false;
                    priming = true;
                    prefilled = 0;
                    let generation = shared
                        .generation
                        .fetch_add(1, Ordering::Relaxed)
                        .wrapping_add(1);
                    history.clear();
                    drop(playback.take());
                    let result =
                        Engine::new(&code, mode, rate, memory_limit_mb).and_then(|engine| {
                            let prepared = pre_render
                                .map(|seconds| PreparedAudio::new(seconds, output_rate))
                                .transpose()?;
                            Ok(Playback { engine, prepared })
                        });
                    let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                    state.generation = generation;
                    state.position = 0;
                    state.frames.clear();
                    state.pre_render = None;
                    state.cached_frames = 0;
                    match result {
                        Ok(new_playback) => {
                            requested_play = play && !new_playback.preparing();
                            new_playback.update_pre_render(&mut state);
                            playback = Some(new_playback);
                            state.error = None;
                            state.playing = requested_play;
                        }
                        Err(error) => {
                            log::error!("Compilation failed: {error}");
                            playback = None;
                            state.error = Some(error);
                            state.playing = false;
                        }
                    }
                }
            }
        }
        if let Some(current) = playback.as_mut() {
            let buffer = shared.buffer_frames.load(Ordering::Relaxed) as usize;
            if requested_play {
                finish_prefill(&shared, &mut priming, prefilled, buffer);
            }
            let available = buffer.saturating_sub(MAX_BUFFER - producer.slots());
            let preparing = current.preparing();
            if preparing || (requested_play && available >= current.next_len()) {
                let result = if preparing {
                    current.prepare_block(output_rate).map(|()| None)
                } else {
                    current.next_block(output_rate).map(Some)
                };
                match result {
                    Ok(block) => {
                        let generation = shared.generation.load(Ordering::Relaxed);
                        if let Some(block) = block {
                            if priming {
                                prefilled += block.frames.len();
                            }
                            for frame in block.frames {
                                // Only this thread produces, and capacity was checked above.
                                producer
                                    .push((frame, generation))
                                    .map_err(|e| format!("Audio queue: {e}"))?;
                                history.push_back(frame);
                                if history.len() > HISTORY {
                                    history.pop_front();
                                }
                            }
                            let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                            state.position = block.position;
                            state.frames = history.iter().copied().collect();
                        }
                        let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                        current.update_pre_render(&mut state);
                    }
                    Err(error) => {
                        log::error!("Rendering failed: {error}");
                        shared.playing.store(false, Ordering::Relaxed);
                        requested_play = false;
                        playback = None;
                        let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
                        state.playing = false;
                        state.error = Some(error);
                        state.pre_render = None;
                        state.cached_frames = 0;
                    }
                }
                continue;
            }
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn finish_prefill(shared: &Shared, priming: &mut bool, prefilled: usize, buffer: usize) {
    if *priming && prefilled > buffer - BLOCK {
        *priming = false;
        shared.playing.store(true, Ordering::Relaxed);
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
                output_error(&shared, &snapshot, format!("Audio output failed: {error}"));
            },
            None,
        )
        .map_err(|e| format!("Opening audio output: {e}"))
}

fn output_error(shared: &Shared, snapshot: &Mutex<Snapshot>, error: String) {
    log::error!("{error}");
    *shared
        .output_error
        .lock()
        .expect("audio output error mutex poisoned") = Some(error.clone());
    shared.playing.store(false, Ordering::Relaxed);
    let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
    state.playing = false;
    state.error = Some(error);
    state.pre_render = None;
    state.cached_frames = 0;
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
    // Discard old songs while paused, but retain queued samples for the current song.
    while consumer
        .peek()
        .is_ok_and(|(_, id)| *id != shared.generation.load(Ordering::Relaxed))
    {
        consumer
            .pop()
            .expect("peeked audio frame must be available");
    }
    if dc_blocker.enabled != enabled || dc_blocker.generation != generation || !playing {
        dc_blocker.reset();
    }
    dc_blocker.enabled = enabled;
    dc_blocker.generation = generation;
    for output_frame in output.chunks_mut(channels) {
        let mut frame = [0.0; 2];
        if playing {
            loop {
                if shared.generation.load(Ordering::Relaxed) != generation
                    || !shared.playing.load(Ordering::Relaxed)
                {
                    break;
                }
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
            buffer_frames: AtomicU32::new(BufferSize::default().frames() as u32),
            output_error: Mutex::new(None),
        }
    }

    fn stateful_playback(code: &str, mode: Mode, rate: f64) -> Playback {
        Playback {
            engine: Engine::new(code, mode, rate, 32).unwrap(),
            prepared: Some(PreparedAudio::new(1, 11025).unwrap()),
        }
    }

    #[test]
    fn pre_render_preserves_state_stereo_and_live_continuation() {
        for (code, mode, rate) in [
            (
                "(t === 0 && (n = 0), [n++ & 255, n * 3 & 255])",
                Mode::Bytebeat,
                8000.0,
            ),
            (
                "return () => { let x = globalThis.n || 0; globalThis.n = x + 1; return [sin(x / 20), cos(x / 31)]; }",
                Mode::Funcbeat,
                35498.66666,
            ),
        ] {
            let mut playback = stateful_playback(code, mode, rate);
            while playback.preparing() {
                playback.prepare_block(11025).unwrap();
            }
            assert_eq!(playback.prepared.as_ref().unwrap().remaining(), 11025);
            let mut actual = Vec::new();
            while actual.len() < 11025 + BLOCK * 4 {
                actual.extend(playback.next_block(11025).unwrap().frames);
            }
            let mut reference = Engine::new(code, mode, rate, 32).unwrap();
            assert_eq!(actual, reference.render(actual.len(), 11025).unwrap());
            assert_eq!(playback.engine.position(), reference.position());
            assert!(playback.prepared.is_none());
        }
    }

    #[test]
    fn cancel_pre_render_retains_completed_samples_without_skipping_state() {
        let code = "(t === 0 && (n = 0), sin(n++ / 20))";
        let mut playback = stateful_playback(code, Mode::Floatbeat, 11025.0);
        for _ in 0..3 {
            playback.prepare_block(11025).unwrap();
        }
        playback.cancel_pre_render();
        assert!(!playback.preparing());
        let mut actual = Vec::new();
        for _ in 0..6 {
            actual.extend(playback.next_block(11025).unwrap().frames);
        }
        let mut reference = Engine::new(code, Mode::Floatbeat, 11025.0, 32).unwrap();
        assert_eq!(actual, reference.render(actual.len(), 11025).unwrap());
    }

    struct Renderer {
        sender: Sender<Command>,
        snapshot: Arc<Mutex<Snapshot>>,
        shared: Arc<Shared>,
        consumer: Consumer<Frame>,
        worker: Option<JoinHandle<Result<(), String>>>,
    }

    impl Renderer {
        fn new() -> Self {
            let (sender, receiver) = mpsc::channel();
            let (producer, consumer) = RingBuffer::new(MAX_BUFFER);
            let snapshot = Arc::new(Mutex::new(Snapshot {
                output_rate: 11025,
                ..Snapshot::default()
            }));
            let shared = Arc::new(shared());
            shared.playing.store(false, Ordering::Relaxed);
            let state = snapshot.clone();
            let controls = shared.clone();
            let worker =
                thread::spawn(move || run_renderer(receiver, state, controls, producer, 11025, 32));
            Self {
                sender,
                snapshot,
                shared,
                consumer,
                worker: Some(worker),
            }
        }

        fn load(&self, code: &str, pre_render: Option<u32>) {
            self.sender
                .send(Command::Load {
                    code: code.into(),
                    mode: Mode::Floatbeat,
                    rate: 11025.0,
                    play: false,
                    pre_render,
                })
                .unwrap();
        }

        fn wait(&self, condition: impl Fn(&Snapshot) -> bool) {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            loop {
                if condition(&self.snapshot.lock().unwrap()) {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "renderer timed out");
                thread::sleep(Duration::from_millis(2));
            }
        }

        fn output(&mut self, frames: usize) -> Vec<f32> {
            let mut output = vec![0.0; frames * 2];
            fill_output(
                &mut output,
                2,
                &mut self.consumer,
                &self.shared,
                &mut DcBlocker::new(11025),
            );
            output
        }
    }

    impl Drop for Renderer {
        fn drop(&mut self) {
            let _ = self.sender.send(Command::Shutdown);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap().unwrap();
            }
        }
    }

    #[test]
    fn queue_size_changes_live_and_playback_waits_for_prefill() {
        let mut renderer = Renderer::new();
        renderer.load("0.4", None);
        renderer.wait(|state| state.generation == 2);
        assert!(!renderer.shared.playing.load(Ordering::Relaxed));
        assert_eq!(renderer.consumer.slots(), 0);
        renderer.sender.send(Command::Play(true)).unwrap();
        for buffer in BufferSize::ALL {
            renderer
                .shared
                .buffer_frames
                .store(buffer.frames() as u32, Ordering::Relaxed);
            renderer.wait(|_| {
                renderer.consumer.slots() == buffer.frames()
                    && renderer.shared.playing.load(Ordering::Relaxed)
            });
            assert!(renderer.shared.playing.load(Ordering::Relaxed));
        }
        renderer.shared.buffer_frames.store(2048, Ordering::Relaxed);
        assert_eq!(
            renderer.output(MAX_BUFFER - 2048),
            vec![0.2; (MAX_BUFFER - 2048) * 2]
        );
        renderer.wait(|_| renderer.consumer.slots() == 2048);
        assert_eq!(renderer.shared.underruns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn lowering_the_buffer_during_prefill_unpauses_existing_queued_audio() {
        let shared = shared();
        shared.playing.store(false, Ordering::Relaxed);
        let (mut producer, mut consumer) = RingBuffer::new(MAX_BUFFER);
        for _ in 0..4096 {
            producer.push(([0.4, -0.6], 1)).unwrap();
        }
        let mut priming = true;
        finish_prefill(&shared, &mut priming, 4096, MAX_BUFFER);
        assert!(priming);
        assert!(!shared.playing.load(Ordering::Relaxed));
        finish_prefill(&shared, &mut priming, 4096, 2048);
        assert!(!priming);
        let mut output = [0.0_f32; 2];
        fill_output(
            &mut output,
            2,
            &mut consumer,
            &shared,
            &mut DcBlocker::new(11025),
        );
        assert_eq!(output, [0.2, -0.3]);
        assert_eq!(consumer.slots(), 4095);
    }

    #[test]
    fn prepared_audio_stays_paused_and_loading_invalidates_cached_and_queued_audio() {
        let mut renderer = Renderer::new();
        renderer.load("0.4", Some(1));
        renderer.wait(|state| {
            state
                .pre_render
                .is_some_and(|progress| progress.rendered_frames == progress.total_frames)
        });
        {
            let state = renderer.snapshot.lock().unwrap();
            assert!(!state.playing);
            assert_eq!(state.position, 0);
            assert_eq!(state.cached_frames, 11025);
        }
        assert_eq!(renderer.output(BLOCK), vec![0.0; BLOCK * 2]);
        assert_eq!(renderer.shared.underruns.load(Ordering::Relaxed), 0);
        renderer.sender.send(Command::Play(true)).unwrap();
        renderer.wait(|_| renderer.shared.playing.load(Ordering::Relaxed));
        assert_eq!(renderer.output(BLOCK), vec![0.2; BLOCK * 2]);
        renderer.load("-0.6", None);
        renderer.wait(|state| state.generation == 3 && !state.playing);
        assert_eq!(renderer.output(BLOCK), vec![0.0; BLOCK * 2]);
        assert_eq!(renderer.consumer.slots(), 0);
        {
            let state = renderer.snapshot.lock().unwrap();
            assert_eq!(state.cached_frames, 0);
            assert!(state.pre_render.is_none());
        }
        renderer.sender.send(Command::Play(true)).unwrap();
        renderer.wait(|_| renderer.shared.playing.load(Ordering::Relaxed));
        assert_eq!(renderer.output(BLOCK), vec![-0.3; BLOCK * 2]);
    }

    #[test]
    fn pre_render_failures_clear_partial_audio_and_allow_recovery() {
        let renderer = Renderer::new();
        for (code, duration) in [
            (
                "t < 128 ? 0.4 : (() => { throw new Error('failed'); })()",
                Some(1),
            ),
            ("0.4", Some(0)),
        ] {
            let generation = renderer.snapshot.lock().unwrap().generation;
            renderer.load(code, duration);
            renderer.wait(|state| state.generation != generation && state.error.is_some());
            let state = renderer.snapshot.lock().unwrap();
            assert!(!state.playing);
            assert_eq!(state.cached_frames, 0);
            assert!(state.pre_render.is_none());
        }
        renderer.load("0.4", Some(1));
        renderer.wait(|state| state.cached_frames == 11025);
        assert!(renderer.snapshot.lock().unwrap().error.is_none());
    }

    #[test]
    fn output_failure_stops_the_renderer_and_clears_prepared_status() {
        let mut renderer = Renderer::new();
        renderer.load("0.4", Some(1));
        renderer.wait(|state| state.cached_frames == 11025);
        output_error(
            &renderer.shared,
            &renderer.snapshot,
            "Audio output failed: disconnected".into(),
        );
        let error = renderer.worker.take().unwrap().join().unwrap().unwrap_err();
        assert_eq!(error, "Audio output failed: disconnected");
        let state = renderer.snapshot.lock().unwrap();
        assert!(!state.playing);
        assert_eq!(state.cached_frames, 0);
        assert!(state.pre_render.is_none());
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
