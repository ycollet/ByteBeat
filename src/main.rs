mod app;
mod audio;
mod catalog;
mod cli;
mod engine;
mod icon;
mod logging;
mod songs;
mod upstream;

use clap::Parser;

fn main() -> std::process::ExitCode {
    logging::init();
    match run(cli::Args::parse()) {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) => {
            log::error!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(args: cli::Args) -> Result<bool, String> {
    args.validate()?;
    log::info!(
        "JavaScript heap limit: {} MiB per song",
        args.memory_limit_mb
    );
    if args.check {
        return cli::check(&args);
    }
    let initial = args
        .file
        .as_ref()
        .map(|path| {
            let entry = args.apply_overrides(catalog::file_entry(path)?);
            log::info!(
                "Opening {} ({}, {} Hz)",
                path.display(),
                entry.metadata.mode.label(),
                entry.metadata.rate
            );
            catalog::load(&entry)
        })
        .transpose()?;
    let (catalog, download_to) = match args.catalog_path()? {
        Some(path) => {
            let songs = catalog::load_songs(&path)?;
            log::info!("Loading {} songs from {}", songs.len(), path.display());
            (songs, None)
        }
        None => (Vec::new(), Some(upstream::cache_root()?)),
    };
    eframe::run_native(
        "Bytebeat Composer",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1240.0, 850.0])
                .with_min_inner_size([800.0, 600.0])
                .with_icon(icon::load()?),
            ..Default::default()
        },
        Box::new(move |cc| {
            Ok(Box::new(app::Composer::new(
                cc,
                initial,
                catalog,
                args.memory_limit_mb,
                args.jack,
                download_to,
            )))
        }),
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}
