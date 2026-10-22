use std::{fs, path::PathBuf, time::Instant};

use clap::Parser;
use serde::Serialize;

use crate::{
    catalog::{self, Entry},
    engine::{Engine, Mode},
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
    /// Load a downloaded catalog; defaults to data/catalog.json if present.
    #[arg(long)]
    pub catalog: Option<PathBuf>,
    /// Override the file's playback mode (or every catalog entry in --check mode).
    #[arg(long, value_enum)]
    pub mode: Option<Mode>,
    /// Override the formula sample rate in Hz; fractional rates are supported.
    #[arg(long)]
    pub sample_rate: Option<f64>,
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
        let default = PathBuf::from("data/catalog.json");
        default
            .try_exists()
            .map(|exists| exists.then_some(default))
            .map_err(|e| format!("data/catalog.json: {e}"))
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
    error: Option<String>,
}

#[derive(Serialize)]
struct Report {
    seconds_per_song: f64,
    output_rate: u32,
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
        return Err("--check requires --file or an available --catalog.".into());
    };
    if entries.is_empty() {
        return Err("The catalog contains no songs to check.".into());
    }
    let mut report = Report {
        seconds_per_song: args.seconds,
        output_rate: 48_000,
        passed: 0,
        failed: 0,
        results: Vec::with_capacity(entries.len()),
    };
    let total_frames = (args.seconds * f64::from(report.output_rate)).ceil() as usize;
    for entry in entries {
        let entry = args.apply_overrides(entry);
        let started = Instant::now();
        let mut row = ResultRow {
            file: entry.file.clone(),
            metadata: entry.metadata.clone(),
            success: false,
            stage: "load",
            samples_checked: 0,
            elapsed_ms: 0,
            peak: 0.0,
            error: None,
        };
        let result = (|| {
            let song = catalog::load(&entry)?;
            row.stage = "compile";
            let mut engine = Engine::new(&song.code, song.mode, song.rate)?;
            row.stage = "render";
            for offset in (0..total_frames).step_by(128) {
                let frames = engine.render((total_frames - offset).min(128), report.output_rate)?;
                row.samples_checked = engine.position() + 1;
                for sample in frames.into_iter().flatten() {
                    row.peak = row.peak.max(sample.abs());
                }
            }
            Ok::<(), String>(())
        })();
        row.elapsed_ms = started.elapsed().as_millis();
        match result {
            Ok(()) => {
                row.success = true;
                row.stage = "ok";
                report.passed += 1;
                log::info!(
                    "PASS {} | {} by {} | {} samples | {} ms",
                    row.file.display(),
                    row.metadata.title,
                    row.metadata.author,
                    row.samples_checked,
                    row.elapsed_ms
                );
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
