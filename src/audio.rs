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
const BLOCK: usize = 128;

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
    pub position: u64,
    pub output_rate: u32,
    pub playing: bool,
    pub error: Option<String>,
}

struct Shared {
    playing: AtomicBool,
    generation: AtomicU64,
    volume: AtomicU32,
    underruns: AtomicU64,
}

pub struct Audio {
    sender: Sender<Command>,
    pub snapshot: Arc<Mutex<Snapshot>>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl Audio {
    pub fn new(memory_limit_mb: u32, use_jack: bool) -> Self {
        let (sender, receiver) = mpsc::channel();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let shared = Arc::new(Shared {
            playing: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            volume: AtomicU32::new(0.25_f32.to_bits()),
            underruns: AtomicU64::new(0),
        });
        let state = snapshot.clone();
        let controls = shared.clone();
        let worker = thread::spawn(move || {
            if let Err(error) = run(receiver, state.clone(), controls, memory_limit_mb, use_jack) {
                log::error!("{error}");
                let mut state = state.lock().expect("audio snapshot mutex poisoned");
                state.playing = false;
                state.error = Some(error);
            }
        });
        Self {
            sender,
            snapshot,
            shared,
            worker: Some(worker),
        }
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

fn run(
    receiver: Receiver<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    shared: Arc<Shared>,
    memory_limit_mb: u32,
    use_jack: bool,
) -> Result<(), String> {
    let host = output_host(use_jack)?;
    log::info!("Audio backend: {}", host.id().name());
    let device = host
        .default_output_device()
        .ok_or_else(|| {
            if use_jack {
                "No JACK output device available. Install libjack, start a JACK server (or enable PipeWire's JACK support), then restart the app.".to_owned()
            } else {
                "No audio output device found. Connect an output device and restart the app.".to_owned()
            }
        })?;
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
                    shared.generation.fetch_add(1, Ordering::Relaxed);
                    history.clear();
                    let result = Engine::new(&code, mode, rate, memory_limit_mb);
                    let mut state = snapshot.lock().expect("audio snapshot mutex poisoned");
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
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                fill_output(output, channels, &mut consumer, &callback_shared);
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
) where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let playing = shared.playing.load(Ordering::Relaxed);
    let generation = shared.generation.load(Ordering::Relaxed);
    let volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
    for output_frame in output.chunks_mut(channels) {
        let mut frame = [0.0; 2];
        if playing {
            loop {
                match consumer.pop() {
                    Ok((samples, id)) if id == generation => {
                        frame = samples;
                        break;
                    }
                    Ok(_) => continue,
                    Err(_) => {
                        shared.underruns.fetch_add(1, Ordering::Relaxed);
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
            *sample = T::from_sample(value * volume);
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
            underruns: AtomicU64::new(0),
        }
    }

    #[test]
    fn pause_preserves_queued_audio_and_resume_plays_it() {
        let shared = shared();
        let (mut producer, mut consumer) = RingBuffer::new(8);
        producer.push(([0.8, -0.4], 1)).unwrap();
        shared.playing.store(false, Ordering::Relaxed);
        let mut output = [1.0_f32; 2];
        fill_output(&mut output, 2, &mut consumer, &shared);
        assert_eq!(output, [0.0, 0.0]);
        assert_eq!(consumer.slots(), 1);
        shared.playing.store(true, Ordering::Relaxed);
        fill_output(&mut output, 2, &mut consumer, &shared);
        assert_eq!(output, [0.4, -0.2]);
    }

    #[test]
    fn restart_discards_old_generations_and_underruns_are_silent() {
        let shared = shared();
        let (mut producer, mut consumer) = RingBuffer::new(8);
        producer.push(([1.0, 1.0], 0)).unwrap();
        producer.push(([0.2, -0.2], 1)).unwrap();
        let mut output = [1.0_f32; 4];
        fill_output(&mut output, 2, &mut consumer, &shared);
        assert_eq!(output, [0.1, -0.1, 0.0, 0.0]);
        assert_eq!(shared.underruns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn mono_downmix_and_surround_channel_silence() {
        let shared = shared();
        let (mut producer, mut consumer) = RingBuffer::new(8);
        producer.push(([0.8, 0.4], 1)).unwrap();
        let mut mono = [0.0_f32; 1];
        fill_output(&mut mono, 1, &mut consumer, &shared);
        assert!((mono[0] - 0.3).abs() < 1e-6);
        producer.push(([0.8, 0.4], 1)).unwrap();
        let mut surround = [1.0_f32; 6];
        fill_output(&mut surround, 6, &mut consumer, &shared);
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
