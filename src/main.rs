// Unter Windows ohne Konsolenfenster starten (der Zeitplan-Lauf protokolliert in last-run.log)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod job;
mod notify;
mod restic;
mod schedule;
mod scheduled;
mod secrets;
mod smb;
mod tray;
mod ui;
mod update;

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "restic-Backup auf NAS (SMB)")]
struct Cli {
    /// Backup ohne GUI ausführen (für Zeitplan)
    #[arg(long)]
    run_backup: bool,
    /// Wie vom Zeitplan: sichert nur, wenn ein Backup fällig ist, keine Pause aktiv und das NAS erreichbar ist
    #[arg(long)]
    scheduled: bool,
    /// Beim Anmelden gestartet: Fenster minimiert öffnen
    #[arg(long)]
    autostart: bool,
}

/// Backup ohne GUI. `quiet_if_busy`: Läuft schon ein Backup, endet der Zeitplan-Lauf still (kein Fehler).
fn run_headless(quiet_if_busy: bool) -> anyhow::Result<()> {
    let cfg = config::Config::load()?;
    let mut lines: Vec<String> = vec![];
    let result = job::run_backup(&cfg, false, &std::sync::atomic::AtomicBool::new(false), |m| lines.push(m), |_| {});
    // Das laufende Backup nicht mit einer Fehlermeldung überschreiben
    if result.as_ref().is_err_and(|e| e.downcast_ref::<job::AlreadyRunning>().is_some()) {
        println!("Es läuft bereits ein Backup.");
        return if quiet_if_busy { Ok(()) } else { result.map(|_| ()) };
    }
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
    if cli.scheduled {
        return scheduled::run();
    }
    if cli.run_backup {
        return run_headless(false);
    }
    // Die App ist kein Spiel: RTSS (MSI Afterburner) hängt seinen Vulkan-Layer sonst in jede Vulkan-
    // Anwendung und blendet das FPS-Overlay ein. Der Layer unterstützt dafür diese Variable. Sie muss
    // vor dem Anlegen der Vulkan-Instanz und vor dem Start weiterer Threads gesetzt werden.
    #[cfg(windows)]
    // SAFETY: läuft ganz am Anfang von `main`, bevor die App weitere Threads startet.
    unsafe {
        std::env::set_var("DISABLE_RTSS_LAYER", "1");
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
            let app = app::App::new(&cc.egui_ctx);
            if cli.autostart {
                // Mit Tray ganz verstecken, sonst minimieren
                let cmd = if app.has_tray() { eframe::egui::ViewportCommand::Visible(false) } else { eframe::egui::ViewportCommand::Minimized(true) };
                cc.egui_ctx.send_viewport_cmd(cmd);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))
}
