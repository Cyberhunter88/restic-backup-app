// Unter Windows ohne Konsolenfenster starten (der Zeitplan-Lauf protokolliert in last-run.log)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod config;
mod job;
mod restic;
mod schedule;
mod secrets;
mod smb;
mod ui;
mod update;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "restic-Backup auf NAS (SMB)")]
struct Cli {
    /// Backup ohne GUI ausführen (für Zeitplan)
    #[arg(long)]
    run_backup: bool,
}

fn run_headless() -> anyhow::Result<()> {
    let cfg = config::Config::load()?;
    let mut lines: Vec<String> = vec![];
    let result = job::run_backup(&cfg, false, &std::sync::atomic::AtomicBool::new(false), |m| lines.push(m), |_| {});
    match &result {
        Ok(_) => lines.push("Backup abgeschlossen.".into()),
        Err(e) => lines.push(format!("FEHLER: {e:#}")),
    }
    let _ = config::write_last_run(result.is_ok(), &lines);
    for l in &lines {
        println!("{l}");
    }
    result.map(|_| ())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    if cli.run_backup {
        return run_headless();
    }
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../packaging/icon.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default().with_inner_size([960.0, 680.0]).with_min_inner_size([760.0, 520.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    eframe::run_native(
        &format!("restic Backup {}", env!("CARGO_PKG_VERSION")),
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(|cc| {
            ui::apply_theme(&cc.egui_ctx);
            Ok(Box::new(app::App::new()))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))
}
