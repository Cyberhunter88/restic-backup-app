// Unter Windows ohne Konsolenfenster starten (der Zeitplan-Lauf protokolliert in last-run.log)
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod autostart;
mod config;
mod job;
mod notify;
mod restic;
mod restore;
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

/// Backup ohne GUI. `scheduled`: Lauf des Zeitplans. Läuft schon ein Backup, endet er still (kein Fehler); ein
/// Fehlschlag zählt für die gestaffelte Wartezeit und wird erst beim zweiten in Folge als Desktop-Meldung
/// gezeigt (eine kurze Störung nach dem Aufwachen ist meist beim nächsten Versuch vorbei).
/// Der Fortschritt geht in `run-status.txt` und ein Abbruch-Wunsch des Fensters (`cancel.flag`) wird beachtet,
/// damit die App einen Zeitplan-Lauf anzeigen und abbrechen kann.
fn run_headless(scheduled: bool) -> anyhow::Result<()> {
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
    // Zuletzt gemeldeter ganzer Prozentwert
    let last_percent = std::cell::Cell::new(-1i64);
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
                let whole = (percent * 100.0).floor() as i64;
                if whole != last_percent.replace(whole) {
                    let mut s = status.borrow_mut();
                    s.percent = percent as f32;
                    config::write_run_status(&s);
                }
            }
        },
    );
    finished.store(true, Ordering::Relaxed);
    // Das laufende Backup nicht mit einer Fehlermeldung überschreiben
    if result.as_ref().is_err_and(|e| e.downcast_ref::<job::AlreadyRunning>().is_some()) {
        if scheduled {
            scheduled::note("Es läuft bereits ein Backup");
            return Ok(());
        }
        println!("Es läuft bereits ein Backup.");
        return result.map(|_| ());
    }
    // Ein Abbruch aus dem Fenster ist kein Fehlschlag: keine Fehlermeldung, keine Wartezeit für den Zeitplan
    let outcome = match &result {
        Ok(_) => config::RunResult::Ok,
        Err(_) if cancel.load(Ordering::Relaxed) => config::RunResult::Cancelled,
        Err(_) => config::RunResult::Failed,
    };
    let error = result.as_ref().err().map(|e| format!("{e:#}"));
    lines.push(match (&result, outcome) {
        (Ok(o), _) => if o.warnings { "Backup mit Warnungen abgeschlossen." } else { "Backup abgeschlossen." }.into(),
        (Err(_), config::RunResult::Cancelled) => "Backup abgebrochen.".to_string(),
        (Err(e), _) => format!("FEHLER: {e:#}"),
    });
    let failures = if scheduled && outcome == config::RunResult::Failed { config::record_schedule_failure() } else { 0 };
    if outcome != config::RunResult::Cancelled && !(scheduled && failures == 1) {
        notify::backup_finished(result.as_ref().is_ok_and(|o| o.warnings), error.as_deref(), cfg.notify_success);
    }
    let _ = config::write_last_run(outcome, &lines);
    for l in &lines {
        println!("{l}");
    }
    if scheduled {
        // Die erste Zeile reicht für das Zeitplan-Protokoll, alles Weitere steht in `last-run.log`
        config::append_schedule_log(&match (&error, outcome) {
            (None, _) => lines.last().cloned().unwrap_or_default(),
            (Some(_), config::RunResult::Cancelled) => "Backup abgebrochen".to_string(),
            (Some(e), _) => format!("Backup fehlgeschlagen ({failures}. Versuch): {}", e.lines().next().unwrap_or(e)),
        });
        if outcome == config::RunResult::Cancelled {
            return Ok(());
        }
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
    // Aus dem Fenster heraus darf die einmalige Einrichtung des Einbindens (Linux) nach dem Passwort fragen
    smb::allow_prompts();
    #[allow(unused_mut)]
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    // Wayland kann kein Fenster verstecken: über XWayland laufen, damit das X die App in den Hintergrund schickt
    #[cfg(target_os = "linux")]
    if ui::prefer_x11() {
        options.event_loop_builder = Some(Box::new(|b| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            b.with_x11();
        }));
    }
    eframe::run_native(
        &format!("restic Backup {}", env!("CARGO_PKG_VERSION")),
        options,
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
                let cmd = if app.has_tray() { ui::hide_command() } else { eframe::egui::ViewportCommand::Minimized(true) };
                cc.egui_ctx.send_viewport_cmd(cmd);
            }
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("GUI-Fehler: {e}"))
}
