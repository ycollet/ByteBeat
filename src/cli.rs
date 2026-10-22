use std::{fs, path::PathBuf, time::Instant};

use clap::Parser;
use serde::Serialize;

use crate::{
    audio::BLOCK,
    catalog::{self, Entry},
    engine::{DEFAULT_MEMORY_LIMIT_MB, Engine, Mode},
    upstream,
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Native bytebeat composer and headless JavaScript song checker"
)]
pub struct Args {
    /// Load a JavaScript file into the editor (reads a matching .json sidecar if present).
    #[arg(short, long)]
    pub file: Option<PathBuf>,
    /// Use a catalog explicitly; otherwise discover a local/installed/cache copy or download upstream.
    #[arg(long)]
    pub catalog: Option<PathBuf>,
    /// Override the file's playback mode (or every catalog entry in --check mode).
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,
    /// Override the formula sample rate in Hz; fractional rates are supported.
    #[arg(long)]
    pub sample_rate: Option<f64>,
    /// JavaScript heap limit per song, in MiB (also applies to GUI playback).
    #[arg(long, default_value_t = DEFAULT_MEMORY_LIMIT_MB, value_parser = clap::value_parser!(u32).range(1..))]
    pub memory_limit_mb: u32,
    /// Use the JACK audio backend instead of the default backend (Linux only).
    #[arg(long, conflicts_with = "check")]
    pub jack: bool,
    /// Compile and render without opening a window or an audio device; exit 1 on failure.
    #[arg(long)]
    pub check: bool,
    /// Duration to render per checked song, in seconds (0.01 to 60).
    #[arg(long, default_value_t = 1.0, requires = "check")]
    pub seconds: f64,
    /// Write a JSON compatibility report, including each failure and sample position.
    #[arg(long, requires = "check")]
    pub report: Option<PathBuf>,
}

impl Args {
    pub fn validate(&self) -> Result<(), String> {
        #[cfg(not(target_os = "linux"))]
        if self.jack {
            return Err("--jack is only supported on Linux.".into());
        }
        if !self.seconds.is_finite() || !(0.01..=60.0).contains(&self.seconds) {
            return Err("--seconds must be between 0.01 and 60.".into());
        }
        if let Some(rate) = self.sample_rate
            && (!rate.is_finite() || !(1_000.0..=192_000.0).contains(&rate))
        {
            return Err("--sample-rate must be between 1000 and 192000 Hz.".into());
        }
        if self.file.is_none() && !self.check && (self.mode.is_some() || self.sample_rate.is_some())
        {
            return Err("--mode and --sample-rate require --file in GUI mode.".into());
        }
        Ok(())
    }

    pub fn catalog_path(&self) -> Result<Option<PathBuf>, String> {
        if let Some(path) = &self.catalog {
            return Ok(Some(path.clone()));
        }
        let local = PathBuf::from("data/catalog.json");
        #[cfg(target_os = "linux")]
        let candidates = [local, PathBuf::from("/usr/share/bytebeat/catalog.json")];
        #[cfg(not(target_os = "linux"))]
        let candidates = [local];
        if let Some(path) = catalog::find_existing(&candidates)? {
            return Ok(Some(path));
        }
        catalog::find_existing(&[upstream::cache_root()?.join("catalog.json")])
    }

    pub fn apply_overrides(&self, mut entry: Entry) -> Entry {
        if let Some(mode) = self.mode {
            entry.metadata.mode = mode;
        }
        if let Some(rate) = self.sample_rate {
            entry.metadata.rate = rate;
        }
        entry
    }
}

#[derive(Serialize)]
struct ResultRow {
    file: PathBuf,
    #[serde(flatten)]
    metadata: catalog::Metadata,
    success: bool,
    stage: &'static str,
    samples_checked: u64,
    elapsed_ms: u128,
    peak: f32,
    thrown_texts: u64,
    performance: Option<RenderPerformance>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RenderPerformance {
    output_frames: usize,
    render_ms: f64,
    realtime_factor: f64,
    block_budget_ms: f64,
    block_p50_ms: f64,
    block_p95_ms: f64,
    block_p99_ms: f64,
    block_max_ms: f64,
    blocks_measured: usize,
    blocks_over_budget: usize,
    over_realtime_budget: bool,
}

impl RenderPerformance {
    fn summarize(
        blocks: &mut [f64],
        output_frames: usize,
        output_rate: u32,
        blocks_over_budget: usize,
    ) -> Self {
        let render_ms: f64 = blocks.iter().sum();
        let audio_ms = output_frames as f64 * 1000.0 / f64::from(output_rate);
        let realtime_factor = render_ms / audio_ms;
        blocks.sort_unstable_by(f64::total_cmp);
        let percentile = |p: usize| blocks[(blocks.len() * p).div_ceil(100) - 1];
        Self {
            output_frames,
            render_ms,
            realtime_factor,
            block_budget_ms: BLOCK as f64 * 1000.0 / f64::from(output_rate),
            block_p50_ms: percentile(50),
            block_p95_ms: percentile(95),
            block_p99_ms: percentile(99),
            block_max_ms: blocks[blocks.len() - 1],
            blocks_measured: blocks.len(),
            blocks_over_budget,
            over_realtime_budget: realtime_factor > 1.0,
        }
    }
}

#[derive(Serialize)]
struct Report {
    seconds_per_song: f64,
    output_rate: u32,
    memory_limit_mb: u32,
    passed: usize,
    failed: usize,
    results: Vec<ResultRow>,
}

pub fn check(args: &Args) -> Result<bool, String> {
    let entries = if let Some(path) = &args.file {
        vec![catalog::file_entry(path)?]
    } else if let Some(path) = args.catalog_path()? {
        catalog::entries(&path)?
    } else {
        let root = upstream::cache_root()?;
        let path = upstream::download(
            &root,
            &std::sync::atomic::AtomicBool::new(false),
            &|message| {
                log::info!("{message}");
                Ok(())
            },
        )?;
        catalog::entries(&path)?
    };
    if entries.is_empty() {
        return Err("The catalog contains no songs to check.".into());
    }
    let mut report = Report {
        seconds_per_song: args.seconds,
        output_rate: 48_000,
        memory_limit_mb: args.memory_limit_mb,
        passed: 0,
        failed: 0,
        results: Vec::with_capacity(entries.len()),
    };
    let total_frames = (args.seconds * f64::from(report.output_rate)).ceil() as usize;
    for entry in entries {
        let entry = args.apply_overrides(entry);
        log::info!(
            "Checking {} | {}",
            entry.file.display(),
            entry.metadata.title
        );
        let started = Instant::now();
        let mut row = ResultRow {
            file: entry.file.clone(),
            metadata: entry.metadata.clone(),
            success: false,
            stage: "load",
            samples_checked: 0,
            elapsed_ms: 0,
            peak: 0.0,
            thrown_texts: 0,
            performance: None,
            error: None,
        };
        let result = (|| {
            let song = catalog::load(&entry)?;
            row.stage = "compile";
            let mut engine = Engine::new(&song.code, song.mode, song.rate, args.memory_limit_mb)?;
            row.stage = "render";
            let mut block_times = Vec::with_capacity(total_frames.div_ceil(BLOCK));
            let mut blocks_over_budget = 0;
            for offset in (0..total_frames).step_by(BLOCK) {
                let block_frames = (total_frames - offset).min(BLOCK);
                let started = Instant::now();
                let rendered = engine.render(block_frames, report.output_rate);
                let render_ms = started.elapsed().as_secs_f64() * 1000.0;
                row.thrown_texts = engine.thrown_texts();
                let frames = rendered?;
                block_times.push(render_ms);
                if render_ms > block_frames as f64 * 1000.0 / f64::from(report.output_rate) {
                    blocks_over_budget += 1;
                }
                row.samples_checked = engine.position() + 1;
                for sample in frames.into_iter().flatten() {
                    row.peak = row.peak.max(sample.abs());
                }
            }
            row.performance = Some(RenderPerformance::summarize(
                &mut block_times,
                total_frames,
                report.output_rate,
                blocks_over_budget,
            ));
            Ok::<(), String>(())
        })();
        row.elapsed_ms = started.elapsed().as_millis();
        match result {
            Ok(()) => {
                row.success = true;
                row.stage = "ok";
                report.passed += 1;
                log::info!(
                    "PASS {} | {} by {} | {} samples | {} ms | {} thrown-text diagnostics",
                    row.file.display(),
                    row.metadata.title,
                    row.metadata.author,
                    row.samples_checked,
                    row.elapsed_ms,
                    row.thrown_texts
                );
                if let Some(performance) = &row.performance {
                    log::info!(
                        "Performance {}: {:.3} ms rendering, {:.3}x real-time cost (lower is faster), block p95 {:.3} ms / {:.3} ms budget, {}/{} blocks over budget",
                        row.file.display(),
                        performance.render_ms,
                        performance.realtime_factor,
                        performance.block_p95_ms,
                        performance.block_budget_ms,
                        performance.blocks_over_budget,
                        performance.blocks_measured,
                    );
                    if performance.over_realtime_budget {
                        log::warn!(
                            "{}: rendering took {:.2}x the audio duration in the checked interval; real-time playback may underrun. Compatibility PASS is unchanged.",
                            row.file.display(),
                            performance.realtime_factor,
                        );
                    }
                }
            }

            Err(error) => {
                log::error!(
                    "FAIL {} | {} by {} | {}: {error}",
                    row.file.display(),
                    row.metadata.title,
                    row.metadata.author,
                    row.stage
                );
                row.error = Some(error);
                report.failed += 1;
            }
        }
        report.results.push(row);
    }
    log::info!(
        "Checked {} songs for {} s each: {} passed, {} failed",
        report.results.len(),
        args.seconds,
        report.passed,
        report.failed
    );
    if let Some(path) = &args.report {
        let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        fs::write(path, json).map_err(|e| format!("{}: {e}", path.display()))?;
        log::info!("Report written to {}", path.display());
    }
    Ok(report.failed == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_render_cost_and_nearest_rank_block_percentiles() {
        let performance = RenderPerformance::summarize(&mut [4.0, 1.0, 3.0, 2.0], 480, 48_000, 2);
        assert_eq!(performance.render_ms, 10.0);
        assert_eq!(performance.output_frames, 480);
        assert_eq!(performance.realtime_factor, 1.0);
        assert!(!performance.over_realtime_budget);
        assert_eq!(performance.block_p50_ms, 2.0);
        assert_eq!(performance.block_p95_ms, 4.0);
        assert_eq!(performance.block_p99_ms, 4.0);
        assert_eq!(performance.block_max_ms, 4.0);
        assert_eq!(performance.blocks_measured, 4);
        assert_eq!(performance.blocks_over_budget, 2);
        assert!((performance.block_budget_ms - 8.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn flags_average_overload_independently_of_individual_block_spikes() {
        let overloaded = RenderPerformance::summarize(&mut [3.0; 4], 480, 48_000, 4);
        assert_eq!(overloaded.realtime_factor, 1.2);
        assert!(overloaded.over_realtime_budget);
        let spike = RenderPerformance::summarize(&mut [4.0, 0.1, 0.1, 0.1], 480, 48_000, 1);
        assert!(spike.realtime_factor < 1.0);
        assert_eq!(spike.blocks_over_budget, 1);
        assert!(!spike.over_realtime_budget);
    }
}
