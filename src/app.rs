use crate::config::Config;
use crate::restic::{BackupEvent, Node, Snapshot};
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
    BackupDone(Result<(), String>),
    Snaps(Result<Vec<Snapshot>, String>),
    Nodes(Result<Vec<Node>, String>),
    Info(Result<String, String>),
    Restored(Result<String, String>),
    RepoInfo(Result<String, String>),
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
    snap_filter: String,
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
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let cfg = Config::load().unwrap_or_default();
        Self {
            cfg,
            tab: Tab::Backup,
            smb_pw: String::new(),
            repo_pw: String::new(),
            new_exclude: String::new(),
            new_marker: String::new(),
            new_time: "12:00".into(),
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
            snap_filter: String::new(),
            selected: None,
            dir: "/".into(),
            nodes: vec![],
            restore_target: String::new(),
            restored_to: None,
            status: String::new(),
            cancel: Arc::new(AtomicBool::new(false)),
        }
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
                    self.log.push(match r {
                        Ok(()) => "Backup erfolgreich.".into(),
                        Err(e) => format!("FEHLER: {e}"),
                    });
                    let _ = crate::config::write_last_run(ok, &self.log);
                    self.last_run = crate::config::read_last_run();
                    self.history = crate::config::read_history(8);
                    self.snaps_loaded = false;
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
                Msg::Info(r) => {
                    self.busy = false;
                    self.status = r.unwrap_or_else(|e| format!("FEHLER: {e}"));
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
            let _ = tx.send(Msg::BackupDone(r.map(|_| ()).map_err(|e| format!("{e:#}"))));
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
                            });
                        }
                        None => ui::badge(ui, "Noch kein Backup", MUTED),
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

        if self.busy && !self.log.is_empty() {
            ui::card(ui, Some("Läuft …"), |ui| {
                if let Some(step) = self.log.iter().rev().find(|l| !l.starts_with("Snapshot")) {
                    ui.label(RichText::new(step).color(MUTED));
                }
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(self.progress).show_percentage().animate(true).desired_width(ui.available_width() - 110.0));
                    if ui.button("Abbrechen").clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
        }

        if !self.history.is_empty() {
            ui::card(ui, Some("Verlauf"), |ui| {
                egui::Grid::new("history").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    for h in &self.history {
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
        ui::card(ui, Some(&format!("Sicherungen ({})", self.snapshots.len())), |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.snap_filter).hint_text("🔍 Filtern nach Datum, Rechner oder Pfad").desired_width(f32::INFINITY));
            let needle = self.snap_filter.to_lowercase();
            egui::ScrollArea::vertical().id_salt("snaps").max_height(190.0).auto_shrink([false, true]).show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    egui::Grid::new("snap_grid").num_columns(4).striped(true).min_col_width(80.0).show(ui, |ui| {
                        for h in ["ID", "Zeitpunkt", "Rechner", "Pfade"] {
                            ui.label(RichText::new(h).color(MUTED).size(13.0));
                        }
                        ui.end_row();
                        for s in &self.snapshots {
                            let when = ui::short_time(&s.time);
                            let paths = s.paths.join(", ");
                            if !needle.is_empty() && ![&when, &s.hostname, &paths, &s.short_id].iter().any(|f| f.to_lowercase().contains(&needle)) {
                                continue;
                            }
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
            self.restore(&ctx, id, include);
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
        egui::ScrollArea::vertical().id_salt("settings_scroll").auto_shrink(false).show(ui, |ui| {
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
                ui::hint(ui, "Muster wie *.tmp oder node_modules");
                removable_list(ui, &mut self.cfg.excludes);
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut self.new_exclude).hint_text("Neues Muster").desired_width(220.0));
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button("Hinzufügen").clicked() || enter) && !self.new_exclude.trim().is_empty() {
                        self.cfg.excludes.push(self.new_exclude.trim().to_string());
                        self.new_exclude.clear();
                    }
                    if ui.button("Übliche Ausschlüsse").clicked() {
                        for p in crate::config::COMMON_EXCLUDES {
                            if !self.cfg.excludes.iter().any(|e| e == p) {
                                self.cfg.excludes.push((*p).to_string());
                            }
                        }
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
            ui::hint(ui, "Neue Versionen stehen auf der Releases-Seite auf GitHub (Anmeldung im Browser erforderlich).");
            if ui.add(ui::primary("Neue Versionen ansehen")).clicked() {
                update::open_url(update::RELEASES_URL);
            }
            ui::hint(ui, "Update: neues Setup (Windows) bzw. .deb/AppImage (Linux) installieren – Einstellungen und Passwörter bleiben erhalten.");
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
        egui::ScrollArea::vertical().id_salt("schedule_scroll").auto_shrink(false).show(ui, |ui| {
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
                        self.status = match self.cfg.validate().and_then(|_| self.cfg.save()).and_then(|_| schedule::install(&self.cfg.schedule_times, &self.cfg.schedule_days)) {
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
                ui::hint(ui, "Verpasste Läufe (Rechner war aus) holt der Zeitplan beim nächsten Start nach. Änderungen gelten erst nach „Zeitplan aktivieren“.");
                #[cfg(windows)]
                ui::hint(ui, "Der Taskplaner-Task läuft unter deinem Benutzer und nutzt die Freigabe direkt per UNC-Pfad.");
            });
            #[cfg(unix)]
            ui::card(ui, Some("Automatisches Einbinden (Linux)"), |ui| {
                ui::hint(ui, "Geplante Läufe können keinen pkexec-Dialog bestätigen. Trage diese Zeile in /etc/fstab ein (als root); danach mountet die App ohne Rückfrage:");
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
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
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
