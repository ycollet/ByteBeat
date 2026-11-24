use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use eframe::egui;
use ffmpeg_sidecar::command::{FfmpegCommand, ffmpeg_is_installed};

use crate::{
    audio::{BLOCK, DcBlocker},
    engine::Engine,
    songs::Song,
};

pub const FPS: u32 = 60;
pub const RATE: u32 = 48000;
const AUDIO_FRAMES: usize = (RATE / FPS) as usize;
const HISTORY: usize = 2048;

pub struct VideoFrame {
    pub history: Vec<[f32; 2]>,
    pub time: f64,
}

struct CaptureWorker {
    song: Song,
    memory_limit_mb: u32,
    volume: f32,
    dc_blocker: bool,
    images: Receiver<egui::ColorImage>,
    video: SyncSender<VideoFrame>,
    shutdown: Arc<AtomicBool>,
}

pub struct Recording {
    pub directory: PathBuf,
    pub frames: u64,
    sender: Option<SyncSender<egui::ColorImage>>,
    receiver: Receiver<VideoFrame>,
    worker: Option<JoinHandle<Result<PathBuf, String>>>,
    shutdown: Arc<AtomicBool>,
}

impl Recording {
    pub fn start(
        parent: &Path,
        mp4: bool,
        song: Song,
        memory_limit_mb: u32,
        volume: f32,
        dc_blocker: bool,
    ) -> Result<Self, String> {
        if parent.as_os_str().is_empty() {
            return Err("Choose an output directory for recordings.".into());
        }
        if mp4 && !ffmpeg_is_installed() {
            return Err("FFmpeg is unavailable. Install ffmpeg on PATH or uncheck Create MP4. No binaries are downloaded automatically.".into());
        }
        fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create recording directory: {e}"))?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("Cannot timestamp recording: {e}"))?
            .as_nanos();
        let directory = parent.join(format!("recording-{timestamp}-{}", std::process::id()));
        fs::create_dir(&directory)
            .map_err(|e| format!("Cannot create recording directory: {e}"))?;
        let wav = WavRecording::new(BufWriter::new(
            File::create_new(directory.join("sound.wav"))
                .map_err(|e| format!("Cannot create recording WAV: {e}"))?,
        ))
        .map_err(|e| format!("Cannot start recording WAV: {e}"))?;
        let (sender, images) = mpsc::sync_channel::<egui::ColorImage>(1);
        let (video, receiver) = mpsc::sync_channel(1);
        let shutdown = Arc::new(AtomicBool::new(false));
        let output = directory.clone();
        let cancelled = shutdown.clone();
        let mut renderer = CaptureWorker {
            song,
            memory_limit_mb,
            volume,
            dc_blocker,
            images,
            video,
            shutdown: cancelled.clone(),
        };
        let worker = thread::Builder::new()
            .name("bytebeat-recording".into())
            .spawn(move || {
                let mut wav = wav;
                let result = renderer.render(&output, &mut wav);
                let finalized = wav.finish();
                match (result, finalized) {
                    (Err(error), finalized) => {
                        let extra = finalized
                            .err()
                            .map(|e| format!("; WAV finalization also failed: {e}"))
                            .unwrap_or_default();
                        return Err(format!(
                            "Recording failed: {error}{extra}. Files retained in {}",
                            output.display()
                        ));
                    }
                    (_, Err(error)) => {
                        return Err(format!(
                            "Cannot finalize recording WAV: {error}. Files retained in {}",
                            output.display()
                        ));
                    }
                    (Ok(count), Ok(())) => {
                        if mp4 && count > 0 && !cancelled.load(Ordering::Relaxed) {
                            encode_mp4(&output, &cancelled).map_err(|e| {
                                format!(
                                    "{e}. PPM frames and sound.wav retained in {}",
                                    output.display()
                                )
                            })?;
                        }
                    }
                }
                Ok(output)
            })
            .map_err(|e| format!("Cannot start recording worker: {e}"))?;
        Ok(Self {
            directory,
            frames: 0,
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            shutdown,
        })
    }
}

impl CaptureWorker {
    fn render(
        &mut self,
        directory: &Path,
        wav: &mut WavRecording<BufWriter<File>>,
    ) -> Result<u64, String> {
        let mut engine = Engine::new(
            &self.song.code,
            self.song.mode,
            self.song.rate,
            self.memory_limit_mb,
        )?;
        let mut filter = DcBlocker::new(RATE);
        let mut history = VecDeque::with_capacity(HISTORY);
        let mut dimensions = None;
        let mut count = 0_u64;
        loop {
            let mut pcm = Vec::with_capacity(AUDIO_FRAMES * 2);
            let mut remaining = AUDIO_FRAMES;
            while remaining > 0 {
                if self.shutdown.load(Ordering::Relaxed) {
                    return Ok(count);
                }
                let block = remaining.min(BLOCK);
                for frame in engine.render(block, RATE)? {
                    history.push_back(frame);
                    if history.len() > HISTORY {
                        history.pop_front();
                    }
                    let frame = if self.dc_blocker {
                        filter.process(frame)
                    } else {
                        frame
                    };
                    pcm.extend(
                        frame
                            .map(|sample| (sample * self.volume).clamp(-1.0, 1.0))
                            .map(|sample| (sample * 32767.0).round() as i16),
                    );
                }
                remaining -= block;
            }
            if self
                .video
                .send(VideoFrame {
                    history: history.iter().copied().collect(),
                    time: (count + 1) as f64 / f64::from(FPS),
                })
                .is_err()
            {
                return Ok(count);
            }
            let image = loop {
                if self.shutdown.load(Ordering::Relaxed) {
                    return Ok(count);
                }
                match self.images.recv_timeout(Duration::from_millis(50)) {
                    Ok(image) => break image,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(count),
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            };
            if let Some(size) = dimensions {
                if image.size != size {
                    return Err("Recording dimensions changed. Keep the window size and display scale unchanged while recording.".into());
                }
            } else {
                dimensions = Some(image.size);
            }
            write_ppm(
                BufWriter::new(
                    File::create_new(directory.join(format!("frame-{count:06}.ppm")))
                        .map_err(|e| format!("Cannot create recording frame: {e}"))?,
                ),
                &image,
            )
            .map_err(|e| format!("Cannot write recording frame: {e}"))?;
            wav.write_samples(&pcm)
                .map_err(|e| format!("Cannot write recording audio: {e}"))?;
            count += 1;
        }
    }
}

impl Recording {
    pub fn is_capturing(&self) -> bool {
        self.sender.is_some()
    }

    pub fn next_frame(&self) -> Option<VideoFrame> {
        // Worker completion reports errors through poll().
        self.receiver.try_recv().ok()
    }

    pub fn crop_visualizer(
        image: &egui::ColorImage,
        rect: egui::Rect,
        scale: f32,
    ) -> Result<egui::ColorImage, String> {
        if !scale.is_finite()
            || scale <= 0.0
            || !rect.is_finite()
            || rect.min.x < 0.0
            || rect.min.y < 0.0
        {
            return Err("Invalid visualizer capture coordinates.".into());
        }
        let min = [(rect.min.x * scale) as usize, (rect.min.y * scale) as usize];
        let max = [(rect.max.x * scale) as usize, (rect.max.y * scale) as usize];
        if max[0] > image.size[0] || max[1] > image.size[1] || max[0] <= min[0] || max[1] <= min[1]
        {
            return Err("Visualizer capture is outside the screenshot. Keep the window size unchanged while recording.".into());
        }
        Ok(image.region(&rect, Some(scale)))
    }

    pub fn capture(&mut self, image: egui::ColorImage) -> Result<(), String> {
        let sender = self.sender.as_ref().ok_or("Recording is finishing.")?;
        sender
            .try_send(image)
            .map_err(|e| format!("Recording capture failed (no frames were skipped): {e}"))?;
        self.frames += 1;
        Ok(())
    }

    pub fn stop(&mut self) {
        self.sender = None;
    }

    pub fn request_shutdown(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        self.stop();
    }

    pub fn poll(&mut self) -> Option<Result<PathBuf, String>> {
        if !self.worker.as_ref()?.is_finished() {
            return None;
        }
        self.stop();
        Some(
            self.worker
                .take()
                .expect("finished recording worker")
                .join()
                .unwrap_or_else(|_| Err("Recording worker panicked.".into())),
        )
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.request_shutdown();
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => log::error!("{error}"),
                Err(_) => log::error!("Recording worker panicked."),
            }
        }
    }
}

fn write_ppm(mut writer: impl Write, image: &egui::ColorImage) -> io::Result<()> {
    if image.size.contains(&0) || image.pixels.len() != image.size[0] * image.size[1] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid recording image",
        ));
    }
    write!(writer, "P6\n{} {}\n255\n", image.size[0], image.size[1])?;
    for pixel in &image.pixels {
        writer.write_all(&pixel.to_array()[..3])?;
    }
    writer.flush()
}

struct WavRecording<W: Write + Seek> {
    writer: W,
    bytes: u32,
}

impl<W: Write + Seek> WavRecording<W> {
    fn new(writer: W) -> io::Result<Self> {
        let mut wav = Self { writer, bytes: 0 };
        wav.header()?;
        Ok(wav)
    }

    fn header(&mut self) -> io::Result<()> {
        self.writer.seek(SeekFrom::Start(0))?;
        self.writer.write_all(b"RIFF")?;
        self.writer.write_all(&(36 + self.bytes).to_le_bytes())?;
        self.writer.write_all(b"WAVEfmt ")?;
        self.writer.write_all(&16_u32.to_le_bytes())?;
        self.writer.write_all(&1_u16.to_le_bytes())?;
        self.writer.write_all(&2_u16.to_le_bytes())?;
        self.writer.write_all(&RATE.to_le_bytes())?;
        self.writer.write_all(&(RATE * 4).to_le_bytes())?;
        self.writer.write_all(&4_u16.to_le_bytes())?;
        self.writer.write_all(&16_u16.to_le_bytes())?;
        self.writer.write_all(b"data")?;
        self.writer.write_all(&self.bytes.to_le_bytes())
    }

    fn write_samples(&mut self, samples: &[i16]) -> io::Result<()> {
        let bytes = samples
            .len()
            .checked_mul(2)
            .and_then(|n| u32::try_from(n).ok())
            .and_then(|n| self.bytes.checked_add(n))
            .filter(|n| *n <= u32::MAX - 36)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "recording exceeds WAV size limit",
                )
            })?;
        for sample in samples {
            self.writer.write_all(&sample.to_le_bytes())?;
        }
        self.bytes = bytes;
        Ok(())
    }

    fn finish(mut self) -> io::Result<()> {
        self.header()?;
        self.writer.flush()
    }
}

fn encode_mp4(directory: &Path, shutdown: &AtomicBool) -> Result<(), String> {
    let mut command = FfmpegCommand::new();
    command
        .no_overwrite()
        .args([
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostats",
            "-framerate",
            "60",
            "-start_number",
            "0",
        ])
        .input("frame-%06d.ppm")
        .input("sound.wav")
        .args([
            "-map",
            "0:v:0",
            "-map",
            "1:a:0",
            "-vf",
            "pad=ceil(iw/2)*2:ceil(ih/2)*2",
        ])
        .codec_video("libx264")
        .args(["-pix_fmt", "yuv420p"])
        .codec_audio("aac")
        .args(["-ar", "48000", "-movflags", "+faststart", "-shortest"])
        .output("recording.mp4");
    let log = File::create_new(directory.join("ffmpeg.log"))
        .map_err(|e| format!("Cannot create FFmpeg log: {e}"))?;
    let mut child = command
        .as_inner_mut()
        .current_dir(directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| format!("Cannot run FFmpeg: {e}"))?;
    let status = loop {
        if shutdown.load(Ordering::Relaxed) {
            let killed = child.kill();
            let waited = child.wait();
            return Err(format!(
                "MP4 conversion cancelled; cleanup: kill={killed:?}, wait={waited:?}"
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(error) => {
                let killed = child.kill();
                let waited = child.wait();
                return Err(format!(
                    "Cannot poll FFmpeg: {error}; cleanup: kill={killed:?}, wait={waited:?}"
                ));
            }
        }
    };
    if !status.success() {
        let mut log = File::open(directory.join("ffmpeg.log")).map_err(|e| e.to_string())?;
        let length = log.metadata().map_err(|e| e.to_string())?.len();
        log.seek(SeekFrom::Start(length.saturating_sub(8192)))
            .map_err(|e| e.to_string())?;
        let mut tail = Vec::new();
        log.read_to_end(&mut tail).map_err(|e| e.to_string())?;
        return Err(format!(
            "FFmpeg exited with {status}: {}",
            String::from_utf8_lossy(&tail).trim()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Mode;

    fn song(mode: Mode, code: &str, rate: f64) -> Song {
        Song {
            title: "Recording test".into(),
            author: "Test".into(),
            code: code.into(),
            mode,
            rate,
        }
    }

    fn image(size: [usize; 2]) -> egui::ColorImage {
        egui::ColorImage::filled(size, egui::Color32::from_rgb(12, 34, 56))
    }

    fn next(recording: &Recording) -> VideoFrame {
        recording
            .receiver
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
    }

    fn finish(mut recording: Recording) -> Result<PathBuf, String> {
        recording.stop();
        recording.worker.take().unwrap().join().unwrap()
    }

    #[test]
    fn records_synchronized_stereo_and_visualizer_frames_without_replaying_state() {
        for (mode, code, rate) in [
            (
                Mode::Bytebeat,
                "(t === 0 && (n = 0), [n++ & 255, n * 3 & 255])",
                8000.0,
            ),
            (
                Mode::Funcbeat,
                "let n = 0; return () => [sin(n++ / 20), cos(n / 30)];",
                35498.66666,
            ),
        ] {
            let parent = tempfile::tempdir().unwrap();
            let mut recording =
                Recording::start(parent.path(), false, song(mode, code, rate), 32, 0.5, false)
                    .unwrap();
            let mut reference = Engine::new(code, mode, rate, 32).unwrap();
            let mut expected = Vec::new();
            for index in 0..4 {
                let frame = next(&recording);
                assert_eq!(frame.time, (index + 1) as f64 / f64::from(FPS));
                let mut audio = Vec::new();
                while audio.len() < AUDIO_FRAMES {
                    audio.extend(
                        reference
                            .render(BLOCK.min(AUDIO_FRAMES - audio.len()), RATE)
                            .unwrap(),
                    );
                }
                let tail = audio.len().min(frame.history.len());
                assert_eq!(
                    &frame.history[frame.history.len() - tail..],
                    &audio[audio.len() - tail..]
                );
                for frame in audio {
                    expected.extend(frame.map(|sample| (sample * 0.5 * 32767.0).round() as i16));
                }
                recording.capture(image([3, 2])).unwrap();
            }
            let directory = finish(recording).unwrap();
            let wav = fs::read(directory.join("sound.wav")).unwrap();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(u16::from_le_bytes(wav[22..24].try_into().unwrap()), 2);
            assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), RATE);
            assert_eq!(
                u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize,
                expected.len() * 2
            );
            assert_eq!(
                &wav[44..],
                expected
                    .iter()
                    .flat_map(|sample| sample.to_le_bytes())
                    .collect::<Vec<_>>()
            );
            for index in 0..4 {
                let ppm = fs::read(directory.join(format!("frame-{index:06}.ppm"))).unwrap();
                assert_eq!(&ppm[..11], b"P6\n3 2\n255\n");
                assert_eq!(&ppm[11..], [12, 34, 56].repeat(6));
            }
            assert!(!directory.join("recording.mp4").exists());
        }
    }

    #[test]
    fn screenshot_crop_uses_requested_scale_and_excludes_gui_pixels() {
        let mut screenshot = image([10, 8]);
        screenshot.pixels[2 * 10 + 4] = egui::Color32::RED;
        let rect = egui::Rect::from_min_max(egui::pos2(2.0, 1.0), egui::pos2(4.0, 3.0));
        let cropped = Recording::crop_visualizer(&screenshot, rect, 2.0).unwrap();
        assert_eq!(cropped.size, [4, 4]);
        assert_eq!(cropped.pixels[0], egui::Color32::RED);
        assert!(Recording::crop_visualizer(&screenshot, rect, 10.0).is_err());
        assert!(Recording::crop_visualizer(&screenshot, rect, f32::NAN).is_err());
        assert!(Recording::crop_visualizer(&screenshot, egui::Rect::NOTHING, 1.0).is_err());
    }

    #[test]
    fn recording_rejects_resize_and_finalizes_previously_accepted_audio() {
        let parent = tempfile::tempdir().unwrap();
        let mut recording = Recording::start(
            parent.path(),
            false,
            song(Mode::Floatbeat, "0.5", 48000.0),
            32,
            1.0,
            false,
        )
        .unwrap();
        let directory = recording.directory.clone();
        next(&recording);
        recording.capture(image([4, 4])).unwrap();
        next(&recording);
        recording.capture(image([6, 4])).unwrap();
        assert!(
            finish(recording)
                .unwrap_err()
                .contains("dimensions changed")
        );
        let wav = fs::read(directory.join("sound.wav")).unwrap();
        assert_eq!(wav.len(), 44 + AUDIO_FRAMES * 4);
        assert!(directory.join("frame-000000.ppm").exists());
        assert!(!directory.join("frame-000001.ppm").exists());
        assert!(!directory.join("recording.mp4").exists());
    }

    #[test]
    fn render_failure_is_reported_and_wav_is_finalized() {
        let parent = tempfile::tempdir().unwrap();
        let recording = Recording::start(
            parent.path(),
            false,
            song(
                Mode::Floatbeat,
                "(() => { throw new Error('recording failed'); })()",
                8000.0,
            ),
            32,
            1.0,
            false,
        )
        .unwrap();
        let directory = recording.directory.clone();
        assert!(finish(recording).unwrap_err().contains("recording failed"));
        assert_eq!(fs::read(directory.join("sound.wav")).unwrap().len(), 44);
    }

    #[test]
    fn recording_applies_dc_blocker_and_volume_without_changing_visualizer_signal() {
        let parent = tempfile::tempdir().unwrap();
        let mut recording = Recording::start(
            parent.path(),
            false,
            song(Mode::Floatbeat, "[0.5, -0.25]", 48000.0),
            32,
            0.5,
            true,
        )
        .unwrap();
        let frame = next(&recording);
        assert!(frame.history.iter().all(|frame| *frame == [0.5, -0.25]));
        recording.capture(image([2, 2])).unwrap();
        let directory = finish(recording).unwrap();
        let wav = fs::read(directory.join("sound.wav")).unwrap();
        let mut filter = DcBlocker::new(RATE);
        let expected: Vec<u8> = (0..AUDIO_FRAMES)
            .flat_map(|_| {
                filter
                    .process([0.5, -0.25])
                    .map(|sample| (sample * 0.5 * 32767.0).round() as i16)
                    .into_iter()
                    .flat_map(i16::to_le_bytes)
            })
            .collect();
        assert_eq!(&wav[44..], expected);
    }

    #[test]
    fn shutdown_without_a_screenshot_finalizes_wav_and_does_not_hang() {
        let parent = tempfile::tempdir().unwrap();
        let mut recording = Recording::start(
            parent.path(),
            false,
            song(Mode::Bytebeat, "t & 255", 8000.0),
            32,
            1.0,
            false,
        )
        .unwrap();
        let directory = recording.directory.clone();
        next(&recording);
        recording.request_shutdown();
        finish(recording).unwrap();
        assert_eq!(fs::read(directory.join("sound.wav")).unwrap().len(), 44);
    }

    #[test]
    fn wav_size_limit_is_checked_before_writing_audio() {
        let mut wav = WavRecording::new(io::Cursor::new(Vec::new())).unwrap();
        wav.bytes = u32::MAX - 37;
        let before = wav.writer.get_ref().clone();
        assert!(wav.write_samples(&[123]).is_err());
        assert_eq!(&before, wav.writer.get_ref());
    }

    #[test]
    #[ignore = "requires installed FFmpeg with libx264/AAC and ffprobe"]
    fn mp4_encoding_retains_sources_and_pads_odd_visualizer_dimensions() {
        let parent = tempfile::tempdir().unwrap();
        let mut recording = Recording::start(
            parent.path(),
            true,
            song(Mode::Floatbeat, "sin(t / 20)", 48000.0),
            32,
            0.25,
            false,
        )
        .unwrap();
        for _ in 0..4 {
            next(&recording);
            recording.capture(image([3, 3])).unwrap();
        }
        let directory = finish(recording).unwrap();
        assert!(fs::metadata(directory.join("recording.mp4")).unwrap().len() > 0);
        assert!(directory.join("sound.wav").exists());
        assert!(directory.join("frame-000003.ppm").exists());
        let probe = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_streams",
                "-of",
                "json",
            ])
            .arg(directory.join("recording.mp4"))
            .output()
            .unwrap();
        assert!(
            probe.status.success(),
            "{}",
            String::from_utf8_lossy(&probe.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        let streams = metadata["streams"].as_array().unwrap();
        let video = streams
            .iter()
            .find(|stream| stream["codec_type"] == "video")
            .unwrap();
        let audio = streams
            .iter()
            .find(|stream| stream["codec_type"] == "audio")
            .unwrap();
        assert_eq!(video["nb_read_frames"], "4");
        assert_eq!(video["width"], 4);
        assert_eq!(video["height"], 4);
        assert_eq!(video["r_frame_rate"], "60/1");
        assert_eq!(audio["channels"], 2);
        assert_eq!(audio["sample_rate"], "48000");
        let original = fs::read(directory.join("recording.mp4")).unwrap();
        assert!(encode_mp4(&directory, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(directory.join("recording.mp4")).unwrap(), original);
    }

    #[test]
    #[ignore = "requires installed FFmpeg"]
    fn mp4_conversion_errors_keep_raw_audio_and_encoder_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let mut wav = WavRecording::new(BufWriter::new(
            File::create_new(directory.path().join("sound.wav")).unwrap(),
        ))
        .unwrap();
        wav.write_samples(&[0; AUDIO_FRAMES * 2]).unwrap();
        wav.finish().unwrap();
        let error = encode_mp4(directory.path(), &AtomicBool::new(false)).unwrap_err();
        assert!(error.contains("FFmpeg exited"), "{error}");
        assert!(directory.path().join("sound.wav").exists());
        assert!(
            fs::metadata(directory.path().join("ffmpeg.log"))
                .unwrap()
                .len()
                > 0
        );
    }
}
