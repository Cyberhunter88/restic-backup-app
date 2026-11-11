//! GUI-Zustand (`App`), Hintergrund-Aufgaben und ihre Meldungen (`Msg`), Seitenleiste. Die Oberfläche der
//! einzelnen Tabs liegt je in einem eigenen Untermodul (`app/<name>_tab.rs`, jeweils ein `impl App`-Block).
mod about_tab;
mod backup_tab;
mod schedule_tab;
mod settings_tab;
mod snapshots_tab;

use crate::config::Config;
use crate::restic::{BackupEvent, DiffEntry, Node, Snapshot};
use crate::secrets::{self, Secret};
use crate::ui::{self, ACCENT, ERR, MUTED, OK, WARN};
#[cfg(unix)]
use crate::smb;
use crate::{job, schedule, update};
use eframe::egui::{self, Frame, Margin, RichText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;

/// Höchstzahl angezeigter Treffer der Dateisuche
const FIND_LIMIT: usize = 500;

enum Msg {
    Log(String),
    Event(BackupEvent),
    BackupDone(Result<bool, String>),
    /// Es läuft schon ein Backup (kein Fehlschlag, wird nicht als solcher verbucht)
    BackupBusy(String),
    Snaps(Result<Vec<Snapshot>, String>),
    Nodes(Result<Vec<Node>, String>),
    /// Treffer der Dateisuche im Snapshot
    Found(Result<Vec<Node>, String>),
    Info(Result<String, String>),
    Restored(Result<String, String>),
    RepoInfo(Result<String, String>),
    Overview(Result<job::RepoOverview, String>),
    Diff(Result<Vec<DiffEntry>, String>),
    /// Ergebnis der Update-Suche: `(Version falls neuer, manuell angestoßen)`
    Update(Result<Option<String>, String>, bool),
    /// Zustand des Zeitplans im System (Task bzw. Timer)
    SchedStatus(schedule::Status),
    /// Ergebnis von „Zeitplan prüfen“
    SchedCheck(String),
    /// „Zeitplan aktivieren“ bzw. „entfernen“ ist fertig (Statustext)
    SchedDone(String),
    /// Fortschritt der Wiederherstellung (0.0 bis 1.0)
    RestoreProgress(f32),
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Backup,
    Snapshots,
    Settings,
    Schedule,
    About,
}

impl Tab {
    const ALL: [(Tab, &'static str, &'static str); 5] = [
        (Tab::Backup, "💾", "Backup"),
        (Tab::Snapshots, "🕓", "Snapshots"),
        (Tab::Settings, "⚙", "Einstellungen"),
        (Tab::Schedule, "⏰", "Zeitplan"),
        (Tab::About, "ℹ", "Info"),
    ];
}

pub struct App {
    cfg: Config,
    tab: Tab,
    smb_pw: String,
    repo_pw: String,
    new_exclude: String,
    new_marker: String,
    new_time: String,
    autostart: bool,
    repo_msg: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: bool,
    /// Ein Backup läuft in einem anderen Prozess (Zeitplan); die App zeigt es an und kann es abbrechen
    external_run: Option<crate::config::RunStatus>,
    external_checked: std::time::Instant,
    /// „Abbrechen“ wurde für den fremden Lauf schon gedrückt
    external_cancel: bool,
    progress: f32,
    /// Text statt Balken, wenn die Gesamtmenge unbekannt ist (`--no-scan`)
    progress_text: String,
    log: Vec<String>,
    /// Protokoll des letzten Laufs (einmal geladen statt bei jedem Frame von der Platte)
    last_run: Option<String>,
    history: Vec<crate::config::HistoryEntry>,
    snapshots: Vec<Snapshot>,
    snaps_loaded: bool,
    overview: Option<job::RepoOverview>,
    overview_msg: String,
    last_check: Option<crate::config::LastCheck>,
    /// Zeitplan pausiert bis (einmal geladen statt bei jedem Frame von der Platte)
    pause_until: Option<chrono::NaiveDateTime>,
    /// Neuere Version auf GitHub (Tag ohne `v`)
    new_version: Option<String>,
    update_msg: String,
    compare_with: Option<String>,
    diff: Option<Vec<DiffEntry>>,
    diff_msg: String,
    /// Nach einem Backup die Repository-Daten neu laden (braucht den egui-Kontext)
    pending_overview: bool,
    tray: Option<crate::tray::Tray>,
    quitting: bool,
    snap_filter: String,
    snap_folder: Option<String>,
    snap_range: ui::SnapRange,
    selected: Option<String>,
    dir: String,
    nodes: Vec<Node>,
    find_term: String,
    /// Treffer der Dateisuche (`None` = normale Ordneransicht)
    found: Option<Vec<Node>>,
    restore_target: String,
    unlock_all: bool,
    restored_to: Option<String>,
    /// Wiederherstellen an den Originalort statt in den Zielordner
    restore_in_place: bool,
    restore_overwrite: crate::restore::Overwrite,
    restore_verify: bool,
    /// Wiederherstellung am Originalort, die noch bestätigt werden muss: `(Snapshot-ID, Pfad oder ganzer Snapshot)`
    confirm_restore: Option<(String, Option<String>)>,
    /// Die laufende Aufgabe lässt sich abbrechen (Wiederherstellung, Prüfung, Suche)
    cancellable: bool,
    /// Fortschritt der laufenden Aufgabe, falls bekannt
    task_progress: Option<f32>,
    /// Nach „Zeitplan aktivieren/entfernen“ die Diagnose neu laden (braucht den egui-Kontext)
    pending_sched_status: bool,
    status: String,
    cancel: Arc<AtomicBool>,
    /// Diagnose im Zeitplan-Tab (einmal geladen statt bei jedem Frame abgefragt)
    sched_status: Option<schedule::Status>,
    sched_check: String,
    sched_log: Vec<(String, String)>,
    sched_last_success: Option<chrono::NaiveDateTime>,
    sched_failures: Option<(u32, chrono::NaiveDateTime)>,
    /// Können geplante Läufe die Freigabe ohne Rückfrage einbinden (Linux: fstab-Eintrag)?
    sched_mount_ready: bool,
    /// Wann zuletzt auf „Backup überfällig“ geprüft wurde
    overdue_checked: std::time::Instant,
}

impl App {
    pub fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let cfg = Config::load().unwrap_or_default();
        let mut app = Self {
            cfg,
            tab: Tab::Backup,
            smb_pw: String::new(),
            repo_pw: String::new(),
            new_exclude: String::new(),
            new_marker: String::new(),
            new_time: "12:00".into(),
            autostart: crate::autostart::is_enabled(),
            repo_msg: String::new(),
            tx,
            rx,
            busy: false,
            external_run: None,
            external_checked: std::time::Instant::now(),
            external_cancel: false,
            progress: 0.0,
            progress_text: String::new(),
            log: vec![],
            last_run: crate::config::read_last_run(),
            history: crate::config::read_history(8),
            snapshots: vec![],
            snaps_loaded: false,
            overview: None,
            overview_msg: String::new(),
            last_check: crate::config::read_last_check(),
            pause_until: crate::config::read_pause(),
            new_version: None,
            update_msg: String::new(),
            compare_with: None,
            diff: None,
            diff_msg: String::new(),
            pending_overview: false,
            tray: crate::tray::start(ctx.clone()),
            quitting: false,
            snap_filter: String::new(),
            snap_folder: None,
            snap_range: ui::SnapRange::All,
            selected: None,
            dir: "/".into(),
            nodes: vec![],
            find_term: String::new(),
            found: None,
            restore_target: String::new(),
            unlock_all: false,
            restored_to: None,
            restore_in_place: false,
            restore_overwrite: crate::restore::Overwrite::Always,
            restore_verify: false,
            confirm_restore: None,
            cancellable: false,
            task_progress: None,
            pending_sched_status: false,
            status: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
            sched_status: None,
            sched_check: String::new(),
            sched_log: vec![],
            sched_last_success: None,
            sched_failures: None,
            sched_mount_ready: true,
            overdue_checked: std::time::Instant::now(),
        };
        // Veralteten Zeitplan (nach Programm-Aktualisierung) still erneuern
        {
            let (times, days, opts) = (app.cfg.schedule_times.clone(), app.cfg.schedule_days, app.sched_opts());
            std::thread::spawn(move || {
                let _ = schedule::refresh_if_outdated(&times, &days, opts);
            });
        }
        // Höchstens einmal täglich nach einer neuen Version suchen
        let due = crate::config::read_update_check().is_none_or(|t| chrono::Local::now().naive_local().signed_duration_since(t).num_hours() >= 24);
        if app.cfg.check_updates && due {
            app.check_update(ctx, false);
        }
        app
    }

    pub fn has_tray(&self) -> bool {
        self.tray.is_some()
    }

    /// Tray-Menü bedienen und das Schließen des Fensters (X) umleiten: Die App läuft im Hintergrund weiter, auch
    /// ohne Tray-Icon. Dann holt ein erneuter Programmstart das Fenster zurück (siehe `main`).
    fn handle_tray(&mut self, ctx: &egui::Context) {
        use crate::tray::TrayCmd;
        let cmds: Vec<TrayCmd> = self.tray.as_ref().map(|t| t.rx.try_iter().collect()).unwrap_or_default();
        for c in cmds {
            match c {
                TrayCmd::Open => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayCmd::Backup if !self.busy && self.external_run.is_none() && self.cfg.is_ready() => self.start_backup(ctx),
                TrayCmd::Backup => {}
                TrayCmd::Pause => self.set_pause(Some(ui::tomorrow_morning(chrono::Local::now().naive_local()))),
                TrayCmd::Resume => self.set_pause(None),
                TrayCmd::Quit => self.quit(ctx),
            }
        }
        if !self.quitting && self.cfg.close_to_tray && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
    }

    /// App wirklich beenden (nicht nur in den Tray schicken). Ein verstecktes Fenster (Wayland) verarbeitet
    /// den Close-Befehl teils nicht; dann beendet der Notausgang nach 10 s den Prozess.
    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        ctx.request_repaint();
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(10));
            std::process::exit(0);
        });
    }

    fn sched_opts(&self) -> schedule::Options {
        schedule::Options { only_on_ac: self.cfg.only_on_ac, wake_to_run: self.cfg.wake_to_run }
    }

    /// Dateien des Zeitplans (Protokoll, letzter Erfolg, Fehlerzähler) für die Diagnose neu lesen.
    fn reload_schedule_files(&mut self) {
        self.sched_log = crate::config::read_schedule_log(8);
        self.sched_last_success = crate::config::read_last_success();
        self.sched_failures = crate::config::read_schedule_failures();
        #[cfg(unix)]
        {
            self.sched_mount_ready = smb::scheduled_mount_ready();
        }
    }

    /// Diagnose neu laden: Dateien sofort, den Zustand im System (Task bzw. Timer) im Hintergrund, ohne `busy` zu setzen.
    fn load_schedule_status(&mut self, ctx: &egui::Context) {
        self.reload_schedule_files();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::SchedStatus(schedule::status()));
            ctx.request_repaint();
        });
    }

    /// „Zeitplan prüfen“: Was täte ein Zeitplan-Lauf jetzt? Startet kein Backup.
    fn check_schedule(&mut self, ctx: &egui::Context) {
        self.sched_check = "Prüfe …".into();
        let (tx, ctx, cfg) = (self.tx.clone(), ctx.clone(), self.cfg.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Msg::SchedCheck(crate::scheduled::dry_run(&cfg)));
            ctx.request_repaint();
        });
    }

    /// Meldet „Backup überfällig“ auch dann, wenn der Zeitplan gar nicht startet (alle 10 Minuten geprüft, die
    /// Meldung selbst erscheint höchstens einmal täglich). Nur bei eingerichtetem Zeitplan und ohne Pause.
    fn check_overdue(&mut self) {
        if self.overdue_checked.elapsed() < std::time::Duration::from_secs(600) {
            return;
        }
        self.overdue_checked = std::time::Instant::now();
        if !self.busy && self.external_run.is_none() && self.active_pause().is_none() && schedule::activated_at().is_some() {
            crate::scheduled::warn_if_overdue(chrono::Local::now().naive_local(), &self.cfg);
        }
    }

    /// Pause des Zeitplans setzen (`None` = fortsetzen) und anzeigen.
    fn set_pause(&mut self, until: Option<chrono::NaiveDateTime>) {
        crate::config::write_pause(until);
        self.pause_until = until;
    }

    /// Aktive Pause (abgelaufene wird ignoriert).
    fn active_pause(&self) -> Option<chrono::NaiveDateTime> {
        self.pause_until.filter(|u| crate::config::pause_active(Some(*u), chrono::Local::now().naive_local()))
    }

    /// Sucht im Hintergrund nach einer neuen Version (ohne `busy` zu setzen).
    fn check_update(&mut self, ctx: &egui::Context, manual: bool) {
        if manual {
            self.update_msg = "Suche …".into();
        }
        crate::config::write_update_check();
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = update::check_latest().map_err(|e| format!("{e:#}"));
            let _ = tx.send(Msg::Update(r, manual));
            ctx.request_repaint();
        });
    }

    /// Startet eine Hintergrund-Aufgabe und sperrt die Oberfläche (`busy`), bis ihre Meldung eintrifft.
    /// Ein alter Abbruch-Wunsch gilt nicht für die neue Aufgabe; ob sie abbrechbar ist, setzt der Aufrufer danach.
    fn spawn(&mut self, ctx: &egui::Context, f: impl FnOnce(Sender<Msg>) + Send + 'static) {
        self.busy = true;
        self.cancel.store(false, Ordering::Relaxed);
        self.cancellable = false;
        self.task_progress = None;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            f(tx);
            ctx.request_repaint();
        });
    }

    /// Sieht nach, ob ein Zeitplan-Lauf (anderer Prozess) gerade sichert. Endet er, werden Verlauf und Übersicht neu geladen.
    fn poll_external_run(&mut self) {
        // Während einer eigenen Aufgabe nicht nachsehen: Hält die App die Sperre selbst (Backup, „Sperren entfernen“),
        // sähe das wie ein fremder Lauf aus. Der letzte Stand bleibt stehen und wird danach aktualisiert.
        if self.busy || self.external_checked.elapsed() < std::time::Duration::from_secs(2) {
            return;
        }
        self.external_checked = std::time::Instant::now();
        let now = crate::config::read_external_run();
        if self.external_run.is_some() && now.is_none() {
            self.reload_schedule_files();
            self.last_run = crate::config::read_last_run();
            self.history = crate::config::read_history(8);
            self.last_check = crate::config::read_last_check();
            self.snaps_loaded = false;
            if self.overview.is_some() {
                self.pending_overview = true;
            }
        }
        if now.is_none() {
            self.external_cancel = false;
        }
        self.external_run = now;
    }

    /// Statustext für eine gescheiterte Aufgabe; ein Abbruch durch den Nutzer ist kein Fehler.
    fn error_text(&self, e: String) -> String {
        if self.cancel.load(Ordering::Relaxed) { "Abgebrochen.".into() } else { format!("FEHLER: {e}") }
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(s) => self.log.push(s),
                Msg::Event(BackupEvent::Progress { percent, files_done, total_files, bytes_done, .. }) => {
                    self.progress = percent as f32;
                    self.progress_text = if total_files == 0 { format!("{files_done} Dateien, {} verarbeitet", ui::human_bytes(bytes_done)) } else { String::new() };
                }
                Msg::Event(BackupEvent::FileError(_)) => {}
                Msg::Event(BackupEvent::Summary { snapshot_id, files_new, data_added }) => {
                    self.log.push(format!("Snapshot {}: {files_new} neue Dateien, {} hinzugefügt", snapshot_id.chars().take(8).collect::<String>(), ui::human_bytes(data_added)));
                    self.progress = 1.0;
                }
                Msg::BackupDone(r) => {
                    self.busy = false;
                    // Ein Abbruch ist kein Fehlschlag: keine Fehlermeldung, im Verlauf als „abgebrochen“
                    let outcome = match &r {
                        Ok(_) => crate::config::RunResult::Ok,
                        Err(_) if self.cancel.load(Ordering::Relaxed) => crate::config::RunResult::Cancelled,
                        Err(_) => crate::config::RunResult::Failed,
                    };
                    if outcome != crate::config::RunResult::Cancelled {
                        crate::notify::backup_finished(matches!(r, Ok(true)), r.as_ref().err().map(String::as_str), self.cfg.notify_success);
                    }
                    self.log.push(match r {
                        Ok(false) => "Backup erfolgreich.".into(),
                        Ok(true) => "Backup mit Warnungen abgeschlossen.".into(),
                        Err(_) if outcome == crate::config::RunResult::Cancelled => "Backup abgebrochen.".into(),
                        Err(e) => format!("FEHLER: {e}"),
                    });
                    let _ = crate::config::write_last_run(outcome, &self.log);
                    self.reload_schedule_files();
                    self.last_run = crate::config::read_last_run();
                    self.history = crate::config::read_history(8);
                    self.last_check = crate::config::read_last_check();
                    self.snaps_loaded = false;
                    if self.overview.is_some() {
                        self.pending_overview = true;
                    }
                }
                Msg::BackupBusy(m) => {
                    self.busy = false;
                    // Meist ein Zeitplan-Lauf, der gerade erst begonnen hat: gleich nachsehen und ihn anzeigen
                    self.external_run = crate::config::read_external_run();
                    self.external_checked = std::time::Instant::now();
                    if self.external_run.is_some() {
                        self.log.clear();
                    } else {
                        self.log.push(format!("FEHLER: {m}"));
                    }
                }
                Msg::Snaps(r) => {
                    self.busy = false;
                    match r {
                        Ok(mut s) => {
                            s.reverse();
                            self.snapshots = s;
                            self.snaps_loaded = true;
                        }
                        Err(e) => self.status = format!("FEHLER: {e}"),
                    }
                }
                Msg::Nodes(r) => {
                    self.busy = false;
                    match r {
                        Ok(n) => self.nodes = n,
                        Err(e) => self.status = format!("FEHLER: {e}"),
                    }
                }
                Msg::Found(r) => {
                    self.busy = false;
                    match r {
                        Ok(n) => {
                            self.status = if n.len() >= FIND_LIMIT { format!("Mehr als {FIND_LIMIT} Treffer – Suchbegriff genauer fassen.") } else { String::new() };
                            self.found = Some(n);
                        }
                        Err(e) => self.status = self.error_text(e),
                    }
                }
                Msg::RepoInfo(r) => {
                    self.busy = false;
                    self.repo_msg = r.unwrap_or_else(|e| format!("FEHLER: {e}"));
                }
                Msg::Diff(r) => {
                    self.busy = false;
                    match r {
                        Ok(d) => {
                            self.diff = Some(d);
                            self.diff_msg.clear();
                        }
                        Err(e) => self.diff_msg = format!("FEHLER: {e}"),
                    }
                }
                Msg::Overview(r) => {
                    self.busy = false;
                    match r {
                        Ok(o) => {
                            self.overview = Some(o);
                            self.overview_msg.clear();
                        }
                        Err(e) => self.overview_msg = format!("FEHLER: {e}"),
                    }
                }
                Msg::Info(r) => {
                    self.busy = false;
                    self.last_check = crate::config::read_last_check();
                    self.status = match r {
                        Ok(m) => m,
                        Err(e) => self.error_text(e),
                    };
                }
                Msg::Update(r, manual) => {
                    match r {
                        Ok(Some(v)) => {
                            self.update_msg = format!("Version {v} ist verfügbar.");
                            self.new_version = Some(v);
                        }
                        Ok(None) => {
                            self.new_version = None;
                            self.update_msg = "Du hast die neueste Version.".into();
                        }
                        Err(e) if manual => self.update_msg = format!("FEHLER: Update-Suche fehlgeschlagen: {e}"),
                        Err(_) => {}
                    }
                }
                Msg::SchedStatus(s) => self.sched_status = Some(s),
                Msg::SchedCheck(m) => self.sched_check = m,
                Msg::SchedDone(m) => {
                    self.busy = false;
                    self.status = m;
                    self.pending_sched_status = true;
                }
                Msg::RestoreProgress(p) => self.task_progress = Some(p),
                Msg::Restored(r) => {
                    self.busy = false;
                    match r {
                        // Leeres Ziel: am Originalort wiederhergestellt (kein einzelner Ordner zum Öffnen)
                        Ok(target) if target.is_empty() => self.status = "Wiederherstellung am Originalort abgeschlossen.".into(),
                        Ok(target) => {
                            self.status = format!("Wiederherstellung nach {target} abgeschlossen.");
                            self.restored_to = Some(target);
                        }
                        Err(e) => self.status = self.error_text(e),
                    }
                }
            }
        }
    }

    fn goto(&mut self, ctx: &egui::Context, tab: Tab) {
        self.tab = tab;
        self.status.clear();
        self.repo_msg.clear();
        if tab == Tab::Snapshots && !self.snaps_loaded && !self.busy && self.cfg.is_ready() {
            self.load_snapshots(ctx);
        }
        if tab == Tab::Schedule {
            self.sched_check.clear();
            self.load_schedule_status(ctx);
        }
    }

    /// Kurztext zum Zeitplan, z. B. „3× täglich“ oder „2× an 5 Tagen“.
    fn schedule_summary(&self) -> String {
        let n = self.cfg.schedule_times.len();
        let days = self.cfg.schedule_days.iter().filter(|d| **d).count();
        match (n, days) {
            (0, _) | (_, 0) => "aus".into(),
            (1, 7) => format!("täglich {}", self.cfg.schedule_times[0]),
            (n, 7) => format!("{n}× täglich"),
            (n, d) => format!("{n}× an {d} Tagen"),
        }
    }

    fn load_snapshots(&mut self, ctx: &egui::Context) {
        let cfg = self.cfg.clone();
        self.spawn(ctx, move |tx| {
            let r = job::open(&cfg).and_then(|r| r.snapshots());
            let _ = tx.send(Msg::Snaps(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn load_overview(&mut self, ctx: &egui::Context) {
        let cfg = self.cfg.clone();
        self.overview_msg = "Lade Repository-Daten …".into();
        self.spawn(ctx, move |tx| {
            let _ = tx.send(Msg::Overview(job::repo_overview(&cfg).map_err(|e| format!("{e:#}"))));
        });
    }

    fn start_backup(&mut self, ctx: &egui::Context) {
        self.progress = 0.0;
        self.progress_text.clear();
        self.log.clear();
        if let Err(e) = self.cfg.validate() {
            self.log.push(format!("FEHLER: {e:#}"));
            return;
        }
        let cfg = self.cfg.clone();
        self.cancel.store(false, Ordering::Relaxed);
        let cancel = self.cancel.clone();
        self.spawn(ctx, move |tx| {
            let (t1, t2) = (tx.clone(), tx.clone());
            let r = job::run_backup(&cfg, true, &cancel, move |m| drop(t1.send(Msg::Log(m))), move |e| drop(t2.send(Msg::Event(e))));
            // „Läuft bereits“ ist kein Fehlschlag: sonst zählte er im Verlauf als Fehler
            if r.as_ref().is_err_and(|e| e.downcast_ref::<job::AlreadyRunning>().is_some()) {
                let _ = tx.send(Msg::BackupBusy(format!("{:#}", r.unwrap_err())));
                return;
            }
            let _ = tx.send(Msg::BackupDone(r.map(|o| o.warnings).map_err(|e| format!("{e:#}"))));
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        egui::Panel::left("nav")
            .exact_size(190.0)
            .resizable(false)
            .frame(Frame::new().fill(ui::SIDEBAR).inner_margin(Margin::same(14)))
            .show(ui, |ui| {
                ui.add_space(6.0);
                ui.label(RichText::new("restic Backup").strong().size(19.0).color(ACCENT));
                ui.add_space(14.0);
                for (t, icon, name) in Tab::ALL {
                    let sel = self.tab == t;
                    let text = RichText::new(format!("{icon}   {name}")).size(15.5);
                    let text = if sel { text.strong().color(egui::Color32::WHITE) } else { text };
                    let b = egui::Button::new(text).selected(sel).frame(sel).fill(if sel { ACCENT.linear_multiply(0.55) } else { egui::Color32::TRANSPARENT }).min_size(egui::vec2(ui.available_width(), 38.0));
                    if ui.add(b).clicked() && !sel {
                        self.goto(&ctx, t);
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(RichText::new(format!("v{}", env!("CARGO_PKG_VERSION"))).color(MUTED).size(12.5));
                    if self.busy {
                        // Punkte statt `ui.spinner()`: der Spinner zeichnet bei jedem Frame neu (Dauer-Last für GPU/CPU)
                        let dots = (ui.input(|i| i.time) * 3.0) as usize % 4;
                        ui.label(RichText::new(format!("Arbeitet {}", ".".repeat(dots))).color(MUTED));
                    }
                });
            });
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // Laufenden Job abbrechen und kurz auf sein Ende warten, bevor die Freigabe getrennt wird
        if self.busy {
            self.cancel.store(true, Ordering::Relaxed);
            for _ in 0..25 {
                self.poll();
                if !self.busy {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        if !self.busy {
            let _ = job::unmount_if_ours(&self.cfg);
        }
    }
}

impl eframe::App for App {
    // `logic` läuft auch bei verstecktem Fenster (Tray): Meldungen der Threads und Tray-Menü werden hier verarbeitet
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll();
        self.poll_external_run();
        // Meldungen der Threads kommen ohne eigenen Repaint an: im Takt nachsehen, solange etwas läuft
        if self.busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            // Einen Zeitplan-Lauf bemerken, auch wenn das Fenster still steht oder versteckt ist
            ctx.request_repaint_after(std::time::Duration::from_secs(if self.external_run.is_some() { 1 } else { 3 }));
        }
        if self.pending_sched_status {
            self.pending_sched_status = false;
            self.load_schedule_status(ctx);
        }
        if self.pending_overview && !self.busy {
            self.pending_overview = false;
            self.load_overview(ctx);
        }
        self.check_overdue();
        self.handle_tray(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.sidebar(ui);
        egui::CentralPanel::default().frame(Frame::new().fill(ui::BG).inner_margin(Margin::same(22))).show(ui, |ui| match self.tab {
            Tab::Backup => self.backup_tab(ui),
            Tab::Snapshots => self.snapshots_tab(ui),
            Tab::Settings => self.settings_tab(ui),
            Tab::Schedule => self.schedule_tab(ui),
            Tab::About => self.about_tab(ui),
        });
    }
}
