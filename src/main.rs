// Unter Windows ohne Konsolenfenster starten (der Zeitplan-Lauf protokolliert in last-run.log)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod job;
mod notify;
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
    /// Beim Anmelden gestartet: Fenster minimiert öffnen
    #[arg(long)]
    autostart: bool,
}

fn run_headless() -> anyhow::Result<()> {
    let cfg = config::Config::load()?;
    let mut lines: Vec<String> = vec![];
    let result = job::run_backup(&cfg, false, &std::sync::atomic::AtomicBool::new(false), |m| lines.push(m), |_| {});
    match &result {
        Ok(o) => lines.push(if o.warnings { "Backup mit Warnungen abgeschlossen." } else { "Backup abgeschlossen." }.into()),
        Err(e) => lines.push(format!("FEHLER: {e:#}")),
    }
    notify::backup_finished(result.as_ref().is_ok_and(|o| o.warnings), result.as_ref().err().map(|e| format!("{e:#}")).as_deref());
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
            if cli.autostart {
                cc.egui_ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(true));
            }
            Ok(Box::new(app::App::new()))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))
}
