use crate::config::Config;
use crate::restic::{BackupEvent, DiffEntry, Node, Snapshot};
use crate::secrets::{self, Secret};
use crate::ui::{self, ACCENT, ERR, MUTED, OK, WARN};
use crate::{job, schedule, update};
#[cfg(unix)]
use crate::smb;
use eframe::egui::{self, Frame, Margin, RichText};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::Arc;

enum Msg {
    Log(String),
    Event(BackupEvent),
    BackupDone(Result<bool, String>),
    Snaps(Result<Vec<Snapshot>, String>),
    Nodes(Result<Vec<Node>, String>),
    Info(Result<String, String>),
    Restored(Result<String, String>),
    RepoInfo(Result<String, String>),
    Overview(Result<job::RepoOverview, String>),
    Diff(Result<Vec<DiffEntry>, String>),
    /// Ergebnis der Update-Suche: `(Version falls neuer, manuell angestoßen)`
    Update(Result<Option<String>, String>, bool),
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
    progress: f32,
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
    restore_target: String,
    restored_to: Option<String>,
    status: String,
    cancel: Arc<AtomicBool>,
}

/// Liste mit Löschknopf je Eintrag.
fn removable_list(ui: &mut egui::Ui, items: &mut Vec<String>) {
    let mut del = None;
    for (i, s) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            if ui.small_button("✖").on_hover_text("Entfernen").clicked() {
                del = Some(i);
            }
            ui.label(RichText::new(s).monospace());
        });
    }
    if let Some(i) = del {
        items.remove(i);
    }
}

/// Beschriftetes Textfeld in einem Grid.
fn field(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).desired_width(340.0).hint_text(hint));
    ui.end_row();
}

fn secret_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(value).password(true).desired_width(340.0).hint_text("leer = unverändert"));
    ui.end_row();
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
            progress: 0.0,
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
            restore_target: String::new(),
            restored_to: None,
            status: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        };
        // Veralteten Zeitplan (nach Programm-Aktualisierung) still erneuern
        {
            let (times, days, opts) = (app.cfg.schedule_times.clone(), app.cfg.schedule_days, schedule::Options { only_on_ac: app.cfg.only_on_ac });
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

    /// Tray-Menü bedienen und das Schließen des Fensters in den Tray umleiten.
    fn handle_tray(&mut self, ctx: &egui::Context) {
        use crate::tray::TrayCmd;
        let Some(tray) = &self.tray else { return };
        let cmds: Vec<TrayCmd> = tray.rx.try_iter().collect();
        for c in cmds {
            match c {
                TrayCmd::Open => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                TrayCmd::Backup if !self.busy && self.cfg.is_ready() => self.start_backup(ctx),
                TrayCmd::Backup => {}
                TrayCmd::Pause => self.set_pause(Some(ui::tomorrow_morning(chrono::Local::now().naive_local()))),
                TrayCmd::Resume => self.set_pause(None),
                TrayCmd::Quit => {
                    self.quitting = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
        if !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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

    fn spawn(&mut self, ctx: &egui::Context, f: impl FnOnce(Sender<Msg>) + Send + 'static) {
        self.busy = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            f(tx);
            ctx.request_repaint();
        });
    }

    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(s) => self.log.push(s),
                Msg::Event(BackupEvent::Progress { percent, .. }) => self.progress = percent as f32,
                Msg::Event(BackupEvent::Summary { snapshot_id, files_new, data_added }) => {
                    self.log.push(format!("Snapshot {}: {files_new} neue Dateien, {} hinzugefügt", snapshot_id.chars().take(8).collect::<String>(), ui::human_bytes(data_added)));
                    self.progress = 1.0;
                }
                Msg::BackupDone(r) => {
                    self.busy = false;
                    let ok = r.is_ok();
                    crate::notify::backup_finished(matches!(r, Ok(true)), r.as_ref().err().map(String::as_str));
                    self.log.push(match r {
                        Ok(false) => "Backup erfolgreich.".into(),
                        Ok(true) => "Backup mit Warnungen abgeschlossen.".into(),
                        Err(e) => format!("FEHLER: {e}"),
                    });
                    let _ = crate::config::write_last_run(ok, &self.log);
                    self.last_run = crate::config::read_last_run();
                    self.history = crate::config::read_history(8);
                    self.last_check = crate::config::read_last_check();
                    self.snaps_loaded = false;
                    if self.overview.is_some() {
                        self.pending_overview = true;
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
                    self.status = r.unwrap_or_else(|e| format!("FEHLER: {e}"));
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
                Msg::Restored(r) => {
                    self.busy = false;
                    match r {
                        Ok(target) => {
                            self.status = format!("Wiederherstellung nach {target} abgeschlossen.");
                            self.restored_to = Some(target);
                        }
                        Err(e) => self.status = format!("FEHLER: {e}"),
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
            let _ = tx.send(Msg::BackupDone(r.map(|o| o.warnings).map_err(|e| format!("{e:#}"))));
        });
    }

    fn backup_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Backup");
        ui.add_space(4.0);
        if !self.cfg.is_ready() {
            ui::card(ui, Some("Einrichtung nötig"), |ui| {
                ui.label("Trage zuerst NAS-Host, Freigabe und mindestens einen Quellordner ein.");
                ui.add_space(6.0);
                if ui.add(ui::primary("Zu den Einstellungen")).clicked() {
                    self.goto(ui.ctx(), Tab::Settings);
                }
            });
            return;
        }

        // Kopfkarte: Status des letzten Laufs + Startknopf
        let ctx = ui.ctx().clone();
        ui::card(ui, None, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Letzter Lauf").color(MUTED).size(13.0));
                    match self.last_run.as_deref().and_then(|l| l.lines().next()) {
                        Some(head) => {
                            let ok = head.contains("ERFOLGREICH");
                            ui.horizontal(|ui| {
                                ui::badge(ui, if ok { "Erfolgreich" } else { "Fehler" }, if ok { OK } else { ERR });
                                ui.label(head.split(" – ").next().unwrap_or(head));
                                if let Some((age, overdue)) = ui::age_text(head, chrono::Local::now().naive_local()) {
                                    ui.label(RichText::new(format!("({age})")).color(if overdue { WARN } else { MUTED }));
                                    if overdue {
                                        ui::badge(ui, "Überfällig", WARN);
                                    }
                                }
                            });
                        }
                        None => ui::badge(ui, "Noch kein Backup", MUTED),
                    }
                    if let Some(until) = self.active_pause() {
                        ui::badge(ui, &format!("Zeitplan pausiert bis {}", until.format("%d.%m. %H:%M")), WARN);
                    }
                    if let Some(v) = &self.new_version {
                        ui::badge(ui, &format!("Neue Version {v} verfügbar (Info-Tab)"), ACCENT);
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_enabled_ui(!self.busy, |ui| {
                        if ui.add(ui::primary("▶  Backup jetzt starten").min_size(egui::vec2(210.0, 44.0))).clicked() {
                            self.start_backup(&ctx);
                        }
                    });
                });
            });
        });

        // Kennzahlen
        ui.columns(3, |cols| {
            let n = self.cfg.sources.len();
            let facts = [
                ("Quellordner".to_string(), format!("{n}")),
                ("Zeitplan".to_string(), self.schedule_summary()),
                ("Aufbewahrung".to_string(), format!("{}T / {}W / {}M", self.cfg.keep_daily, self.cfg.keep_weekly, self.cfg.keep_monthly)),
            ];
            for (col, (label, value)) in cols.iter_mut().zip(facts) {
                ui::card(col, None, |ui| {
                    ui.label(RichText::new(label).color(MUTED).size(13.0));
                    ui.label(RichText::new(value).strong().size(19.0));
                });
            }
        });

        if self.cfg.is_ready() {
            ui::card(ui, Some("Repository"), |ui| {
                ui.horizontal_wrapped(|ui| {
                    match &self.overview {
                        Some(o) => {
                            ui.label(format!("Größe: {}", ui::human_bytes(o.size)));
                            ui.label(format!("Snapshots: {}", o.snapshots));
                            if let Some(f) = o.free {
                                ui.label(format!("Frei auf dem NAS: {}", ui::human_bytes(f)));
                            }
                        }
                        None => ui::hint(ui, "Noch nicht geladen (verbindet kurz mit dem NAS)."),
                    }
                    if ui.add_enabled(!self.busy, egui::Button::new(if self.overview.is_some() { "⟳ Aktualisieren" } else { "Laden" })).clicked() {
                        self.load_overview(&ctx);
                    }
                });
                ui::status(ui, &self.overview_msg);
                let now = chrono::Local::now().naive_local();
                let due = crate::config::check_due(self.last_check.as_ref(), self.cfg.check_interval_days, now);
                match &self.last_check {
                    Some(c) => {
                        let text = format!("Letzte Prüfung: {}{}", c.time.format("%d.%m.%Y %H:%M"), if c.ok { "" } else { " – FEHLGESCHLAGEN" });
                        ui.label(RichText::new(text).color(if !c.ok { ERR } else if due { WARN } else { MUTED }).size(13.0));
                    }
                    None if self.cfg.check_interval_days > 0 => ui::hint(ui, "Noch keine Prüfung durchgeführt (läuft automatisch im Zeitplan-Lauf)."),
                    None => {}
                }
            });
        }

        if self.busy && !self.log.is_empty() {
            // Aktueller Schritt als Titel; das Protokoll darunter zeigt ihn nicht doppelt
            let title = self.log.iter().rev().find(|l| !l.starts_with("Snapshot")).cloned().unwrap_or_else(|| "Läuft …".into());
            ui::card(ui, Some(&title), |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(self.progress).show_percentage().desired_width(ui.available_width() - 110.0));
                    if ui.button("Abbrechen").clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
        }

        // Der neueste Lauf steht schon oben unter „Letzter Lauf“
        if self.history.len() > 1 {
            ui::card(ui, Some("Frühere Läufe"), |ui| {
                egui::Grid::new("history").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    for h in self.history.iter().skip(1) {
                        ui.label(RichText::new(&h.time).color(MUTED));
                        if h.ok { ui::badge(ui, "OK", OK) } else { ui::badge(ui, "Fehler", ERR) }
                        ui.label(&h.summary);
                        ui.end_row();
                    }
                });
            });
        }

        ui::card(ui, Some("Protokoll"), |ui| {
            let from_disk: Vec<String>;
            let lines: &[String] = if self.log.is_empty() {
                from_disk = self.last_run.as_deref().map(|l| l.lines().skip(1).map(String::from).collect()).unwrap_or_default();
                &from_disk
            } else {
                &self.log
            };
            if lines.is_empty() {
                ui::hint(ui, "Noch nichts protokolliert. Starte ein Backup, um den Verlauf zu sehen.");
            }
            egui::ScrollArea::vertical().id_salt("log").stick_to_bottom(true).auto_shrink([false, true]).max_height(ui.available_height().max(120.0)).show(ui, |ui| {
                for l in lines {
                    let color = if l.starts_with("FEHLER") { ERR } else if l.starts_with("Warnung") { WARN } else { ui.visuals().text_color() };
                    ui.label(RichText::new(l).monospace().color(color));
                }
            });
        });
    }

    fn snapshots_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Snapshots");
        ui.add_space(4.0);
        let ctx = ui.ctx().clone();
        ui::card(ui, None, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(!self.busy, egui::Button::new("⟳  Neu laden")).clicked() {
                    self.load_snapshots(&ctx);
                }
                if ui.add_enabled(!self.busy, egui::Button::new("🔍  Repository prüfen")).on_hover_text("restic check").clicked() {
                    let cfg = self.cfg.clone();
                    self.status = "Prüfe Repository …".into();
                    self.spawn(&ctx, move |tx| {
                        let r = job::open(&cfg).and_then(|r| r.check());
                        crate::config::write_last_check(r.is_ok());
                        let _ = tx.send(Msg::Info(r.map(|_| "Repository ist in Ordnung.".to_string()).map_err(|e| format!("{e:#}"))));
                    });
                }
                if self.busy {
                    ui.spinner();
                }
                ui::status(ui, &self.status);
            });
            ui.horizontal(|ui| {
                ui.label("Wiederherstellen nach:");
                ui.add(egui::TextEdit::singleline(&mut self.restore_target).desired_width(ui.available_width() - 110.0).hint_text("Zielordner wählen"));
                if ui.button("Wählen …").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.restore_target = p.to_string_lossy().into_owned();
                    }
                }
            });
            if let Some(path) = self.restored_to.clone() {
                if ui.link(format!("📂 {path} im Dateimanager öffnen")).clicked() {
                    update::open_url(&path);
                }
            }
        });

        let busy = self.busy;
        let mut to_select = None;
        let now = chrono::Local::now();
        let mut folders: Vec<String> = self.snapshots.iter().flat_map(|s| s.paths.iter().cloned()).collect();
        folders.sort();
        folders.dedup();
        if self.snap_folder.as_ref().is_some_and(|f| !folders.contains(f)) {
            self.snap_folder = None;
        }
        let shown = self.snapshots.iter().filter(|s| ui::snapshot_matches(s, &self.snap_filter, self.snap_folder.as_deref(), self.snap_range, now)).count();
        let title = if shown == self.snapshots.len() { format!("Sicherungen ({shown})") } else { format!("Sicherungen ({shown} von {})", self.snapshots.len()) };
        ui::card(ui, Some(&title), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.snap_filter).hint_text("🔍 Datum, Rechner oder Pfad").desired_width(220.0));
                egui::ComboBox::from_id_salt("snap_folder").selected_text(self.snap_folder.clone().unwrap_or_else(|| "Alle Ordner".into())).width(220.0).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.snap_folder, None, "Alle Ordner");
                    for f in &folders {
                        ui.selectable_value(&mut self.snap_folder, Some(f.clone()), f);
                    }
                });
                for (r, name) in ui::SnapRange::ALL {
                    ui.selectable_value(&mut self.snap_range, r, name);
                }
            });
            egui::ScrollArea::vertical().id_salt("snaps").max_height(190.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::Grid::new("snap_grid").num_columns(4).striped(true).min_col_width(80.0).show(ui, |ui| {
                        for h in ["ID", "Zeitpunkt", "Rechner", "Pfade"] {
                            ui.label(RichText::new(h).color(MUTED).size(13.0));
                        }
                        ui.end_row();
                        for s in &self.snapshots {
                            if !ui::snapshot_matches(s, &self.snap_filter, self.snap_folder.as_deref(), self.snap_range, now) {
                                continue;
                            }
                            let when = ui::short_time(&s.time);
                            let paths = s.paths.join(", ");
                            let sel = self.selected.as_deref() == Some(&s.id);
                            if ui.selectable_label(sel, RichText::new(&s.short_id).monospace()).clicked() {
                                to_select = Some(s.id.clone());
                            }
                            ui.label(when);
                            ui.label(&s.hostname);
                            ui.label(RichText::new(paths).color(MUTED));
                            ui.end_row();
                        }
                    });
                });
            });
            if self.snapshots.is_empty() && !self.busy {
                ui::hint(ui, "Keine Snapshots gefunden.");
            }
        });
        if let Some(id) = to_select {
            self.selected = Some(id);
            self.compare_with = None;
            self.diff = None;
            self.diff_msg.clear();
            self.dir = "/".into();
            self.load_dir(&ctx);
        }
        let Some(id) = self.selected.clone() else {
            ui::hint(ui, "Wähle einen Snapshot, um seine Dateien zu durchsuchen.");
            return;
        };

        let mut enter = None;
        let mut restore = None;
        let can_restore = !self.busy && !self.restore_target.is_empty();
        ui::card(ui, Some("Dateien im Snapshot"), |ui| {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.dir != "/" && !self.busy, egui::Button::new("↑")).on_hover_text("Eine Ebene hoch").clicked() {
                    let parent = std::path::Path::new(&self.dir).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or("/".into());
                    self.dir = if parent.is_empty() { "/".into() } else { parent };
                    self.load_dir(&ctx);
                }
                ui.label(RichText::new(&self.dir).monospace());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add_enabled(can_restore, egui::Button::new("Ganzen Snapshot wiederherstellen")).on_disabled_hover_text("Zuerst ein Ziel wählen").clicked() {
                        restore = Some((None::<String>, true));
                    }
                });
            });
            egui::ScrollArea::vertical().id_salt("nodes").auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("node_grid").num_columns(3).striped(true).min_col_width(90.0).show(ui, |ui| {
                    for n in &self.nodes {
                        if n.kind == "dir" {
                            if ui.add_enabled(!self.busy, egui::Button::new(format!("📁 {}", n.name)).frame(false)).clicked() {
                                enter = Some(n.path.clone());
                            }
                            ui.label("");
                        } else {
                            ui.label(&n.name);
                            ui.label(RichText::new(ui::human_bytes(n.size)).color(MUTED));
                        }
                        if ui.add_enabled(can_restore, egui::Button::new("Wiederherstellen")).on_disabled_hover_text("Zuerst ein Ziel wählen").clicked() {
                            restore = Some((Some(n.path.clone()), false));
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if let Some(d) = enter {
            self.dir = d;
            self.load_dir(&ctx);
        }
        if let Some((include, _)) = restore {
            self.restore(&ctx, id.clone(), include);
        }
        self.diff_card(ui, &ctx, &id);
    }

    /// Vergleicht den gewählten Snapshot mit einem anderen (`restic diff`).
    fn diff_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, id: &str) {
        let others: Vec<(String, String)> = self.snapshots.iter().filter(|s| s.id != id).map(|s| (s.id.clone(), format!("{}  {}", s.short_id, ui::short_time(&s.time)))).collect();
        let busy = self.busy;
        let mut run = false;
        ui::card(ui, Some("Änderungen gegenüber einem anderen Snapshot"), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Vergleichen mit");
                let shown = others.iter().find(|(i, _)| Some(i) == self.compare_with.as_ref()).map(|(_, l)| l.clone()).unwrap_or_else(|| "Snapshot wählen …".into());
                egui::ComboBox::from_id_salt("compare_with").selected_text(shown).width(240.0).show_ui(ui, |ui| {
                    for (i, l) in &others {
                        ui.selectable_value(&mut self.compare_with, Some(i.clone()), l);
                    }
                });
                if ui.add_enabled(!busy && self.compare_with.is_some(), egui::Button::new("Vergleichen")).clicked() {
                    run = true;
                }
            });
            ui::status(ui, &self.diff_msg);
            if let Some(d) = &self.diff {
                const MAX: usize = 500;
                let (add, del, chg) = (d.iter().filter(|e| e.modifier == "+").count(), d.iter().filter(|e| e.modifier == "-").count(), d.iter().filter(|e| e.modifier != "+" && e.modifier != "-").count());
                ui.label(format!("{add} nur im gewählten Snapshot (+), {del} nur im anderen (−), {chg} geändert"));
                if d.is_empty() {
                    ui::hint(ui, "Keine Unterschiede.");
                }
                egui::ScrollArea::vertical().id_salt("diff").max_height(220.0).auto_shrink([false, true]).show(ui, |ui| {
                    for e in d.iter().take(MAX) {
                        let color = match e.modifier.as_str() {
                            "+" => OK,
                            "-" => ERR,
                            _ => WARN,
                        };
                        ui.label(RichText::new(format!("{}  {}", if e.modifier == "-" { "−" } else { e.modifier.as_str() }, e.path)).monospace().color(color));
                    }
                    if d.len() > MAX {
                        ui::hint(ui, &format!("… und {} weitere", d.len() - MAX));
                    }
                });
            }
        });
        if run {
            if let Some(other) = self.compare_with.clone() {
                let (cfg, base) = (self.cfg.clone(), id.to_string());
                self.diff = None;
                self.diff_msg = "Vergleiche …".into();
                self.spawn(ctx, move |tx| {
                    let r = job::open(&cfg).and_then(|r| r.diff(&other, &base));
                    let _ = tx.send(Msg::Diff(r.map_err(|e| format!("{e:#}"))));
                });
            }
        }
    }

    fn load_dir(&mut self, ctx: &egui::Context) {
        let (cfg, id, dir) = (self.cfg.clone(), self.selected.clone().unwrap_or_default(), self.dir.clone());
        self.spawn(ctx, move |tx| {
            let r = job::open(&cfg).and_then(|r| r.ls(&id, &dir));
            let _ = tx.send(Msg::Nodes(r.map_err(|e| format!("{e:#}"))));
        });
    }

    fn restore(&mut self, ctx: &egui::Context, id: String, include: Option<String>) {
        let (cfg, target) = (self.cfg.clone(), self.restore_target.clone());
        self.status = "Stelle wieder her …".into();
        self.restored_to = None;
        self.spawn(ctx, move |tx| {
            let r = job::open(&cfg).and_then(|r| r.restore(&id, &target, include.as_deref()));
            let _ = tx.send(Msg::Restored(r.map(|_| target).map_err(|e| format!("{e:#}"))));
        });
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        // Speichern-Leiste bleibt unten sichtbar, der Rest scrollt
        egui::Panel::bottom("save_bar").frame(Frame::new().fill(ui::BG).inner_margin(Margin::symmetric(0, 10))).show(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.add(ui::primary("Speichern")).clicked() {
                    self.status = match self.save() {
                        Ok(()) => "Gespeichert.".into(),
                        Err(e) => format!("FEHLER: {e:#}"),
                    };
                }
                ui::status(ui, &self.status);
            });
        });
        ui.heading("Einstellungen");
        ui.add_space(4.0);
        if self.busy {
            ui::hint(ui, "Gesperrt, solange ein Lauf aktiv ist.");
        }
        let locked = self.busy;
        egui::ScrollArea::vertical().id_salt("settings_scroll").auto_shrink(false).show(ui, |ui| {
            ui.add_enabled_ui(!locked, |ui| {
            ui::card(ui, Some("NAS-Verbindung"), |ui| {
                egui::Grid::new("nas").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    field(ui, "NAS-Host", &mut self.cfg.host, "z. B. nas.local");
                    field(ui, "Freigabe", &mut self.cfg.share, "Name der SMB-Freigabe");
                    field(ui, "SMB-Benutzer", &mut self.cfg.smb_user, "");
                    secret_field(ui, "SMB-Passwort", &mut self.smb_pw);
                });
            });

            ui::card(ui, Some("Repository"), |ui| {
                egui::Grid::new("repo").num_columns(2).spacing([14.0, 10.0]).show(ui, |ui| {
                    field(ui, "Unterordner", &mut self.cfg.subpath, "Ordner des restic-Repos in der Freigabe");
                    secret_field(ui, "Repo-Passwort", &mut self.repo_pw);
                    field(ui, "restic-Programm", &mut self.cfg.restic_bin, "restic");
                });
                ui.checkbox(&mut self.cfg.allow_init, "Neues Repo anlegen, falls keins existiert");
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("Repo prüfen")).clicked() {
                        let cfg = self.cfg.clone();
                        self.repo_msg = "Prüfe Repository …".into();
                        self.spawn(ui.ctx(), move |tx| {
                            let _ = tx.send(Msg::RepoInfo(job::describe_repo(&cfg).map_err(|e| format!("{e:#}"))));
                        });
                    }
                    if self.busy {
                        ui.spinner();
                    }
                    ui::status(ui, &self.repo_msg);
                });
                ui::hint(ui, "Bestehendes Repo verwenden: Freigabe und Unterordner wie im Repo angeben, Passwort eintragen, dann „Repo prüfen“.");
            });

            ui::card(ui, Some("Quellordner"), |ui| {
                if self.cfg.sources.is_empty() {
                    ui::hint(ui, "Noch keine Ordner ausgewählt.");
                }
                removable_list(ui, &mut self.cfg.sources);
                ui.checkbox(&mut self.cfg.separate_snapshots, "Jeden Ordner als eigenen Snapshot sichern");
                ui::hint_long(ui, "hint_sep", "Je Ordner ein Snapshot; die Aufbewahrung gilt dann je Ordner.", "Z. B. getrennte Snapshots für C:\\Users\\admin und C:\\Users\\xyz. Bereits vorhandene gemeinsame Snapshots bleiben bestehen und müssen bei Bedarf manuell gelöscht werden.");
                if ui.button("＋  Ordner hinzufügen …").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        let p = p.to_string_lossy().into_owned();
                        if !self.cfg.sources.contains(&p) {
                            self.cfg.sources.push(p);
                        }
                    }
                }
            });

            ui::card(ui, Some("Ausschlüsse"), |ui| {
                ui::hint(ui, "Muster wie *.tmp oder **/node_modules – eine Liste mit einem Muster pro Zeile kann direkt eingefügt werden");
                removable_list(ui, &mut self.cfg.excludes);
                ui.add(egui::TextEdit::multiline(&mut self.new_exclude).hint_text("Muster hier einfügen (ein Muster pro Zeile)").desired_rows(3).desired_width(f32::INFINITY));
                ui.horizontal(|ui| {
                    if ui.button("Hinzufügen").clicked() && !self.new_exclude.trim().is_empty() {
                        crate::config::add_excludes(&mut self.cfg.excludes, &self.new_exclude);
                        self.new_exclude.clear();
                    }
                    if ui.button("Übliche Ausschlüsse").clicked() {
                        crate::config::add_excludes(&mut self.cfg.excludes, &crate::config::COMMON_EXCLUDES.join("\n"));
                    }
                });
                ui.separator();
                ui.checkbox(&mut self.cfg.exclude_caches, "Ordner mit CACHEDIR.TAG ausschließen");
                ui.horizontal(|ui| {
                    ui.label("Dateien größer als");
                    ui.add(egui::TextEdit::singleline(&mut self.cfg.exclude_larger_than).hint_text("z. B. 500M").desired_width(90.0));
                    ui::hint(ui, "leer = kein Limit");
                });
                ui.separator();
                ui.label("Ordner mit Markerdatei ausschließen");
                removable_list(ui, &mut self.cfg.exclude_if_present);
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_marker).hint_text(".nobackup").desired_width(220.0));
                    if ui.button("Marker hinzufügen").clicked() && !self.new_marker.trim().is_empty() {
                        self.cfg.exclude_if_present.push(self.new_marker.trim().to_string());
                        self.new_marker.clear();
                    }
                });
            });

            ui::card(ui, Some("Aufbewahrung"), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Täglich");
                    ui.add(egui::DragValue::new(&mut self.cfg.keep_daily).range(0..=999));
                    ui.label("Wöchentlich");
                    ui.add(egui::DragValue::new(&mut self.cfg.keep_weekly).range(0..=999));
                    ui.label("Monatlich");
                    ui.add(egui::DragValue::new(&mut self.cfg.keep_monthly).range(0..=999));
                });
                ui::hint(ui, "So viele Snapshots pro Zeitraum bleiben erhalten, ältere werden nach jedem Backup entfernt.");
            });

            ui::card(ui, Some("Integritätsprüfung"), |ui| {
                ui.horizontal(|ui| {
                    ui.label("Alle");
                    ui.add(egui::DragValue::new(&mut self.cfg.check_interval_days).range(0..=365));
                    ui.label("Tage");
                    ui.label("Anteil der Daten");
                    ui.add(egui::DragValue::new(&mut self.cfg.check_subset_percent).range(1..=100).suffix(" %"));
                });
                ui::hint(ui, "Der Zeitplan-Lauf prüft nach dem Backup einen Teil der Repository-Daten (0 Tage = aus). Ein Fehler wird als Warnung gemeldet.");
            });
            });
        });
    }

    fn save(&mut self) -> anyhow::Result<()> {
        self.cfg.validate()?;
        self.cfg.save()?;
        if !self.smb_pw.is_empty() {
            secrets::set(Secret::Smb, &self.smb_pw)?;
            self.smb_pw.clear();
        }
        if !self.repo_pw.is_empty() {
            secrets::set(Secret::Repo, &self.repo_pw)?;
            self.repo_pw.clear();
        }
        Ok(())
    }

    fn about_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Info");
        ui.add_space(4.0);
        ui::card(ui, Some("restic Backup"), |ui| {
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.horizontal(|ui| {
                if ui.button("Nach Updates suchen").clicked() {
                    let ctx = ui.ctx().clone();
                    self.check_update(&ctx, true);
                }
                if ui.add(ui::primary("Neue Versionen ansehen")).clicked() {
                    update::open_url(update::RELEASES_URL);
                }
            });
            ui::status(ui, &self.update_msg);
            ui.checkbox(&mut self.cfg.check_updates, "Beim Start täglich nach einer neuen Version suchen");
            ui::hint_long(ui, "hint_update", "Die Suche fragt nur die öffentliche GitHub-Release-Seite ab und sendet keine Daten.", "Update: neues Setup (Windows) bzw. .deb/AppImage (Linux) installieren – Einstellungen und Passwörter bleiben erhalten. Danach den Zeitplan einmal neu aktivieren. Die Option wird mit „Speichern“ (Einstellungen) übernommen.");
        });
        ui::card(ui, Some("Konfiguration"), |ui| {
            ui::hint(ui, "Einstellungen als Datei sichern oder auf einen anderen Rechner übernehmen. Passwörter sind nicht enthalten.");
            ui.horizontal(|ui| {
                if ui.add_enabled(!self.busy, egui::Button::new("Exportieren …")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().set_file_name("restic-backup-einstellungen.toml").add_filter("TOML", &["toml"]).save_file() {
                        self.status = match self.cfg.export_to(&p) {
                            Ok(()) => format!("Exportiert nach {}.", p.display()),
                            Err(e) => format!("FEHLER: {e:#}"),
                        };
                    }
                }
                if ui.add_enabled(!self.busy, egui::Button::new("Importieren …")).clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("TOML", &["toml"]).pick_file() {
                        self.status = match Config::import_from(&p) {
                            Ok(c) => {
                                self.cfg = c;
                                "Importiert, aber noch nicht gespeichert. Unter „Einstellungen“ speichern und die Passwörter neu eintragen.".into()
                            }
                            Err(e) => format!("FEHLER: {e:#}"),
                        };
                    }
                }
            });
            ui::status(ui, &self.status);
        });
        ui::card(ui, Some("Dateien"), |ui| {
            if let Ok(p) = crate::config::config_path() {
                ui.label(RichText::new(format!("Konfiguration: {}", p.display())).monospace().size(12.5));
            }
            if let Ok(p) = crate::config::last_run_path() {
                ui.label(RichText::new(format!("Letztes Protokoll: {}", p.display())).monospace().size(12.5));
            }
        });
    }

    fn schedule_tab(&mut self, ui: &mut egui::Ui) {
        ui.heading("Zeitplan");
        ui.add_space(4.0);
        if self.busy {
            ui::hint(ui, "Gesperrt, solange ein Lauf aktiv ist.");
        }
        let locked = self.busy;
        egui::ScrollArea::vertical().id_salt("schedule_scroll").auto_shrink(false).show(ui, |ui| {
            ui.add_enabled_ui(!locked, |ui| {
            ui::card(ui, Some("Wann soll gesichert werden?"), |ui| {
                ui.label("Schnellauswahl");
                ui.horizontal_wrapped(|ui| {
                    for (name, times) in [
                        ("1× täglich", &["02:00"][..]),
                        ("2× täglich", &["08:00", "20:00"]),
                        ("Alle 6 Stunden", &["00:00", "06:00", "12:00", "18:00"]),
                        ("Alle 3 Stunden", &["00:00", "03:00", "06:00", "09:00", "12:00", "15:00", "18:00", "21:00"]),
                        ("Stündlich", &["00:00", "01:00", "02:00", "03:00", "04:00", "05:00", "06:00", "07:00", "08:00", "09:00", "10:00", "11:00", "12:00", "13:00", "14:00", "15:00", "16:00", "17:00", "18:00", "19:00", "20:00", "21:00", "22:00", "23:00"]),
                    ] {
                        if ui.button(name).clicked() {
                            self.cfg.schedule_times = times.iter().map(|t| t.to_string()).collect();
                        }
                    }
                });
                ui.separator();
                ui.label("Uhrzeiten");
                let mut del = None;
                ui.horizontal_wrapped(|ui| {
                    for (i, t) in self.cfg.schedule_times.iter().enumerate() {
                        if ui.button(format!("{t}  ✖")).on_hover_text("Entfernen").clicked() {
                            del = Some(i);
                        }
                    }
                });
                if let Some(i) = del {
                    self.cfg.schedule_times.remove(i);
                }
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.new_time).hint_text("HH:MM").desired_width(80.0));
                    if ui.button("＋ Uhrzeit hinzufügen").clicked() {
                        match schedule::parse_time(self.new_time.trim()) {
                            Ok((h, m)) => {
                                let t = format!("{h:02}:{m:02}");
                                if !self.cfg.schedule_times.contains(&t) {
                                    self.cfg.schedule_times.push(t);
                                    self.cfg.schedule_times.sort();
                                }
                                self.status.clear();
                            }
                            Err(_) => self.status = "FEHLER: Uhrzeit bitte als HH:MM angeben (z. B. 14:30).".into(),
                        }
                    }
                });
                ui.separator();
                ui.label("Wochentage");
                ui.horizontal(|ui| {
                    for (i, d) in ["Mo", "Di", "Mi", "Do", "Fr", "Sa", "So"].iter().enumerate() {
                        ui.toggle_value(&mut self.cfg.schedule_days[i], *d);
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.add(ui::primary("Zeitplan aktivieren")).clicked() {
                        self.status = match self.cfg.validate().and_then(|_| self.cfg.save()).and_then(|_| schedule::install(&self.cfg.schedule_times, &self.cfg.schedule_days, schedule::Options { only_on_ac: self.cfg.only_on_ac })) {
                            Ok(()) => format!("Zeitplan aktiv: {}.", self.schedule_summary()),
                            Err(e) => format!("FEHLER: {e:#}"),
                        };
                    }
                    if ui.button("Zeitplan entfernen").clicked() {
                        self.status = match schedule::remove() {
                            Ok(()) => "Zeitplan entfernt.".into(),
                            Err(e) => format!("FEHLER: {e:#}"),
                        };
                    }
                });
                ui::status(ui, &self.status);
                ui::hint_long(
                    ui,
                    "hint_catchup",
                    "Änderungen gelten erst nach „Zeitplan aktivieren“. Verpasste Läufe werden nachgeholt.",
                    "Der Zeitplan startet das Programm zu den Uhrzeiten, bei der Anmeldung, beim Entsperren und alle 30 Minuten; gesichert wird nur, wenn seit dem letzten geplanten Termin noch kein Backup gelang. War der Laptop aus, wird also einmal nachgeholt (auch nach Tagen). Ist das NAS nicht erreichbar (z. B. unterwegs), wird der Lauf still übersprungen und später erneut versucht – ohne Fehler im Protokoll. Schlägt ein Lauf fehl, wartet der Zeitplan zwei Stunden bis zum nächsten Versuch. Nach einer Programm-Aktualisierung wird der Zeitplan beim Start der App automatisch erneuert.",
                );
                #[cfg(windows)]
                ui::hint(ui, "Der Taskplaner-Task läuft unter deinem Benutzer und nutzt die Freigabe direkt per UNC-Pfad.");
            });
            ui::card(ui, Some("Laptop und Pause"), |ui| {
                ui.checkbox(&mut self.cfg.only_on_ac, "Nur am Netzteil sichern");
                ui.horizontal(|ui| {
                    ui.label("Warnen nach");
                    ui.add(egui::DragValue::new(&mut self.cfg.warn_after_days).range(0..=60).suffix(" Tagen"));
                    ui.label("ohne Backup (0 = aus)");
                });
                ui::hint(ui, "Gilt nach „Zeitplan aktivieren“ bzw. „Speichern“. Die Warnung erscheint als Desktop-Meldung, höchstens einmal täglich.");
                ui.separator();
                match self.active_pause() {
                    Some(until) => {
                        ui.label(RichText::new(format!("Zeitplan pausiert bis {}", until.format("%d.%m.%Y %H:%M"))).color(WARN));
                    }
                    None => {
                        ui.label("Der Zeitplan ist aktiv.");
                    }
                }
                ui.horizontal(|ui| {
                    let now = chrono::Local::now().naive_local();
                    if ui.button("Bis morgen früh pausieren").clicked() {
                        self.set_pause(Some(ui::tomorrow_morning(now)));
                    }
                    if ui.button("1 Woche pausieren").clicked() {
                        self.set_pause(Some(now + chrono::Duration::days(7)));
                    }
                    if ui.add_enabled(self.active_pause().is_some(), egui::Button::new("Fortsetzen")).clicked() {
                        self.set_pause(None);
                    }
                });
            });
            ui::card(ui, Some("Autostart"), |ui| {
                if ui.checkbox(&mut self.autostart, "Programm beim Anmelden automatisch starten (minimiert)").changed() {
                    self.status = match crate::autostart::set(self.autostart) {
                        Ok(()) if self.autostart => "Autostart eingerichtet.".into(),
                        Ok(()) => "Autostart entfernt.".into(),
                        Err(e) => {
                            self.autostart = crate::autostart::is_enabled();
                            format!("FEHLER: {e:#}")
                        }
                    };
                }
                ui::hint(ui, "Der Zeitplan läuft unabhängig vom Autostart.");
                #[cfg(unix)]
                ui::hint_long(ui, "hint_linger", "Linux: Der Timer läuft nur bei angemeldetem Benutzer.", "Für Backups auch ohne Anmeldung: sudo loginctl enable-linger $USER");
            });
            #[cfg(unix)]
            ui::card(ui, Some("Automatisches Einbinden (Linux)"), |ui| {
                ui::hint_long(ui, "hint_fstab", "Geplante Läufe brauchen einen fstab-Eintrag (als root).", "Geplante Läufe können keinen pkexec-Dialog bestätigen. Trage diese Zeile in /etc/fstab ein (als root); danach mountet die App ohne Rückfrage:");
                let mut line = smb::fstab_line(&self.cfg).unwrap_or_default();
                ui.add(egui::TextEdit::multiline(&mut line).code_editor().desired_rows(3).desired_width(f32::INFINITY));
                if ui.button("Credentials-Datei anlegen (SMB-Passwort aus Keyring)").clicked() {
                    self.status = match secrets::get(Secret::Smb).and_then(|p| p.ok_or_else(|| anyhow::anyhow!("Kein SMB-Passwort gespeichert"))).and_then(|p| smb::write_credentials(&self.cfg, &p)) {
                        Ok(()) => "Credentials-Datei (0600) angelegt.".into(),
                        Err(e) => format!("FEHLER: {e:#}"),
                    };
                }
            });
            });
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
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Arbeitet …").color(MUTED));
                        });
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
        if self.pending_overview && !self.busy {
            self.pending_overview = false;
            self.load_overview(ctx);
        }
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
