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
/// Der Fortschritt geht in `run-status.txt` und ein Abbruch-Wunsch des Fensters (`cancel.flag`) wird beachtet,
/// damit die App einen Zeitplan-Lauf anzeigen und abbrechen kann.
fn run_headless(quiet_if_busy: bool) -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let cfg = config::Config::load()?;
    let mut lines: Vec<String> = vec![];
    let cancel = Arc::new(AtomicBool::new(false));
    // Erst wenn der Lauf die Sperre hält (erste Protokollzeile), gehören Status und Abbruch-Datei ihm; ein zweiter
    // Prozess, der nur „läuft bereits“ meldet, darf weder den Status überschreiben noch den Abbruch wegnehmen
    let holding = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    {
        let (cancel, holding, finished) = (cancel.clone(), holding.clone(), finished.clone());
        std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                if holding.load(Ordering::Relaxed) && config::take_cancel_request() {
                    cancel.store(true, Ordering::Relaxed);
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });
    }
    let status = std::cell::RefCell::new(config::RunStatus { started: Some(chrono::Local::now().naive_local()), ..Default::default() });
    let result = job::run_backup(
        &cfg,
        false,
        &cancel,
        |m| {
            holding.store(true, Ordering::Relaxed);
            status.borrow_mut().text.clone_from(&m);
            config::write_run_status(&status.borrow());
            lines.push(m);
        },
        |e| {
            // Nur bei ganzen Prozentschritten schreiben (restic meldet mehrmals pro Sekunde)
            if let restic::BackupEvent::Progress { percent, .. } = e {
                let mut s = status.borrow_mut();
                if (percent * 100.0).floor() != f64::from(s.percent * 100.0).floor() {
                    s.percent = percent as f32;
                    config::write_run_status(&s);
                }
            }
        },
    );
    finished.store(true, Ordering::Relaxed);
    // Das laufende Backup nicht mit einer Fehlermeldung überschreiben
    if result.as_ref().is_err_and(|e| e.downcast_ref::<job::AlreadyRunning>().is_some()) {
        println!("Es läuft bereits ein Backup.");
        return if quiet_if_busy { Ok(()) } else { result.map(|_| ()) };
    }
    match &result {
        Ok(o) => lines.push(if o.warnings { "Backup mit Warnungen abgeschlossen." } else { "Backup abgeschlossen." }.into()),
        Err(e) => lines.push(format!("FEHLER: {e:#}")),
    }
    notify::backup_finished(result.as_ref().is_ok_and(|o| o.warnings), result.as_ref().err().map(|e| format!("{e:#}")).as_deref(), cfg.notify_success);
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
    // Nur ein Fenster: ein zweiter Start holt das laufende Fenster nach vorn und beendet sich
    let _instance = match config::InstanceLock::acquire() {
        Ok(Some(l)) => Some(l),
        Ok(None) => {
            if !cli.autostart {
                config::request_show_window();
            }
            println!("restic Backup läuft bereits.");
            return Ok(());
        }
        // Sperre nicht möglich (z. B. Datenordner nicht schreibbar): trotzdem starten
        Err(_) => None,
    };
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../packaging/icon.png")).ok();
    let mut viewport = eframe::egui::ViewportBuilder::default().with_app_id("restic-backup-app").with_inner_size([960.0, 680.0]).with_min_inner_size([760.0, 520.0]);
    if let Some(icon) = icon {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }
    eframe::run_native(
        &format!("restic Backup {}", env!("CARGO_PKG_VERSION")),
        eframe::NativeOptions { viewport, ..Default::default() },
        Box::new(|cc| {
            ui::apply_theme(&cc.egui_ctx);
            let app = app::App::new(&cc.egui_ctx);
            // Zweiter Start meldet sich über eine Datei: Fenster zeigen (auch aus dem Tray)
            let ctx = cc.egui_ctx.clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                    if config::take_show_window_request() {
                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Visible(true));
                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Minimized(false));
                        ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Focus);
                        ctx.request_repaint();
                    }
                }
            });
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
