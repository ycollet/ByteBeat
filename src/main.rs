mod app;
mod audio;
mod catalog;
mod cli;
mod engine;
mod songs;

use clap::Parser;

fn main() -> std::process::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
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
    let catalog = args
        .catalog_path()?
        .map(|path| {
            let entries = catalog::entries(&path)?;
            log::info!("Loading {} songs from {}", entries.len(), path.display());
            entries
                .iter()
                .map(catalog::load)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    eframe::run_native(
        "Bytebeat Composer",
        eframe::NativeOptions {
            viewport: eframe::egui::ViewportBuilder::default()
                .with_inner_size([1240.0, 850.0])
                .with_min_inner_size([800.0, 600.0]),
            ..Default::default()
        },
        Box::new(move |cc| Ok(Box::new(app::Composer::new(cc, initial, catalog)))),
    )
    .map_err(|e| e.to_string())?;
    Ok(true)
}
