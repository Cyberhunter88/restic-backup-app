use crate::config::Config;
use crate::restic::{BackupEvent, Node, Snapshot};
use crate::secrets::{self, Secret};
use crate::{job, schedule, smb};
use eframe::egui;
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
}

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Backup,
    Snapshots,
    Settings,
    Schedule,
}

pub struct App {
    cfg: Config,
    tab: Tab,
    smb_pw: String,
    repo_pw: String,
    new_exclude: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    busy: bool,
    progress: f32,
    log: Vec<String>,
    snapshots: Vec<Snapshot>,
    selected: Option<String>,
    dir: String,
    nodes: Vec<Node>,
    restore_target: String,
    status: String,
    cancel: Arc<AtomicBool>,
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
            tx,
            rx,
            busy: false,
            progress: 0.0,
            log: vec![],
            snapshots: vec![],
            selected: None,
            dir: "/".into(),
            nodes: vec![],
            restore_target: String::new(),
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
                    self.log.push(format!("Snapshot {snapshot_id}: {files_new} neue Dateien, {data_added} Bytes hinzugefügt"));
                    self.progress = 1.0;
                }
                Msg::BackupDone(r) => {
                    self.busy = false;
                    self.log.push(match r {
                        Ok(()) => "Backup erfolgreich.".into(),
                        Err(e) => format!("FEHLER: {e}"),
                    });
                }
                Msg::Snaps(r) => {
                    self.busy = false;
                    match r {
                        Ok(mut s) => {
                            s.reverse();
                            self.snapshots = s;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Msg::Nodes(r) => {
                    self.busy = false;
                    match r {
                        Ok(n) => self.nodes = n,
                        Err(e) => self.status = e,
                    }
                }
                Msg::Info(r) => {
                    self.busy = false;
                    self.status = r.unwrap_or_else(|e| format!("FEHLER: {e}"));
                }
            }
        }
    }

    fn backup_tab(&mut self, ui: &mut egui::Ui) {
        if !self.cfg.is_ready() {
            ui.label("Bitte zuerst unter „Einstellungen“ NAS und Quellordner eintragen.");
            return;
        }
        ui.add_enabled_ui(!self.busy, |ui| {
            if ui.button("Backup jetzt starten").clicked() {
                self.progress = 0.0;
                self.log.clear();
                let cfg = self.cfg.clone();
                self.cancel.store(false, Ordering::Relaxed);
                let cancel = self.cancel.clone();
                self.spawn(ui.ctx(), move |tx| {
                    let (t1, t2) = (tx.clone(), tx.clone());
                    let r = job::run_backup(&cfg, true, &cancel, move |m| drop(t1.send(Msg::Log(m))), move |e| drop(t2.send(Msg::Event(e))));
                    let _ = tx.send(Msg::BackupDone(r.map_err(|e| format!("{e:#}"))));
                });
            }
        });
        if self.busy {
            ui.horizontal(|ui| {
                ui.add(egui::ProgressBar::new(self.progress).show_percentage());
                if ui.button("Abbrechen").clicked() {
                    self.cancel.store(true, Ordering::Relaxed);
                }
            });
        }
        egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
            for l in &self.log {
                ui.label(l);
            }
        });
    }

    fn snapshots_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!self.busy, |ui| {
                if ui.button("Snapshots laden").clicked() {
                    let cfg = self.cfg.clone();
                    self.spawn(ui.ctx(), move |tx| {
                        let r = job::open(&cfg).and_then(|r| r.snapshots());
                        let _ = tx.send(Msg::Snaps(r.map_err(|e| format!("{e:#}"))));
                    });
                }
            });
            if self.busy {
                ui.spinner();
            }
            ui.label(&self.status);
        });
        ui.horizontal(|ui| {
            ui.label("Wiederherstellen nach:");
            ui.text_edit_singleline(&mut self.restore_target);
            if ui.button("Wählen …").clicked() {
                if let Some(p) = rfd::FileDialog::new().pick_folder() {
                    self.restore_target = p.to_string_lossy().into_owned();
                }
            }
        });
        ui.separator();
        let mut to_select = None;
        egui::ScrollArea::vertical().id_salt("snaps").max_height(160.0).show(ui, |ui| {
            for s in &self.snapshots {
                let sel = self.selected.as_deref() == Some(&s.id);
                if ui.selectable_label(sel, format!("{}  {}  {}  {}", s.short_id, s.time, s.hostname, s.paths.join(", "))).clicked() {
                    to_select = Some(s.id.clone());
                }
            }
        });
        if let Some(id) = to_select {
            self.selected = Some(id);
            self.dir = "/".into();
            self.load_dir(ui.ctx());
        }
        let Some(id) = self.selected.clone() else { return };
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(format!("Ordner: {}", self.dir));
            if self.dir != "/" && ui.button("↑ Hoch").clicked() {
                let parent = std::path::Path::new(&self.dir).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or("/".into());
                self.dir = if parent.is_empty() { "/".into() } else { parent };
                self.load_dir(ui.ctx());
            }
            if ui.add_enabled(!self.busy && !self.restore_target.is_empty(), egui::Button::new("Gesamten Snapshot wiederherstellen")).clicked() {
                self.restore(ui.ctx(), id.clone(), None);
            }
        });
        let mut enter = None;
        let mut restore = None;
        egui::ScrollArea::vertical().id_salt("nodes").show(ui, |ui| {
            for n in &self.nodes {
                ui.horizontal(|ui| {
                    if n.kind == "dir" {
                        if ui.link(format!("📁 {}", n.name)).clicked() {
                            enter = Some(n.path.clone());
                        }
                    } else {
                        ui.label(format!("{} ({} B)", n.name, n.size));
                    }
                    if ui.add_enabled(!self.busy && !self.restore_target.is_empty(), egui::Button::new("Wiederherstellen")).clicked() {
                        restore = Some(n.path.clone());
                    }
                });
            }
        });
        if let Some(d) = enter {
            self.dir = d;
            self.load_dir(ui.ctx());
        }
        if let Some(p) = restore {
            self.restore(ui.ctx(), id, Some(p));
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
        self.spawn(ctx, move |tx| {
            let r = job::open(&cfg).and_then(|r| r.restore(&id, &target, include.as_deref()));
            let _ = tx.send(Msg::Info(r.map(|_| format!("Wiederherstellung nach {target} abgeschlossen")).map_err(|e| format!("{e:#}"))));
        });
    }

    fn settings_tab(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("settings").num_columns(2).show(ui, |ui| {
            ui.label("NAS-Host:");
            ui.text_edit_singleline(&mut self.cfg.host);
            ui.end_row();
            ui.label("Freigabe:");
            ui.text_edit_singleline(&mut self.cfg.share);
            ui.end_row();
            ui.label("Unterordner (Repo):");
            ui.text_edit_singleline(&mut self.cfg.subpath);
            ui.end_row();
            ui.label("SMB-Benutzer:");
            ui.text_edit_singleline(&mut self.cfg.smb_user);
            ui.end_row();
            ui.label("SMB-Passwort:");
            ui.add(egui::TextEdit::singleline(&mut self.smb_pw).password(true).hint_text("leer = unverändert"));
            ui.end_row();
            ui.label("Repo-Passwort:");
            ui.add(egui::TextEdit::singleline(&mut self.repo_pw).password(true).hint_text("leer = unverändert"));
            ui.end_row();
            ui.label("restic-Programm:");
            ui.text_edit_singleline(&mut self.cfg.restic_bin);
            ui.end_row();
            ui.label("Aufbewahren (täglich/wöchentl./monatl.):");
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut self.cfg.keep_daily));
                ui.add(egui::DragValue::new(&mut self.cfg.keep_weekly));
                ui.add(egui::DragValue::new(&mut self.cfg.keep_monthly));
            });
            ui.end_row();
        });
        ui.separator();
        ui.label("Quellordner:");
        let mut del = None;
        for (i, s) in self.cfg.sources.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✖").clicked() {
                    del = Some(i);
                }
                ui.label(s);
            });
        }
        if let Some(i) = del {
            self.cfg.sources.remove(i);
        }
        if ui.button("Ordner hinzufügen …").clicked() {
            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                self.cfg.sources.push(p.to_string_lossy().into_owned());
            }
        }
        ui.separator();
        ui.label("Ausschlüsse (Muster, z. B. *.tmp oder node_modules):");
        let mut del = None;
        for (i, s) in self.cfg.excludes.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✖").clicked() {
                    del = Some(i);
                }
                ui.label(s);
            });
        }
        if let Some(i) = del {
            self.cfg.excludes.remove(i);
        }
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.new_exclude);
            if ui.button("Hinzufügen").clicked() && !self.new_exclude.trim().is_empty() {
                self.cfg.excludes.push(self.new_exclude.trim().to_string());
                self.new_exclude.clear();
            }
        });
        ui.separator();
        if ui.button("Speichern").clicked() {
            self.status = match self.save() {
                Ok(()) => "Gespeichert.".into(),
                Err(e) => format!("FEHLER: {e:#}"),
            };
        }
        ui.label(&self.status);
    }

    fn save(&mut self) -> anyhow::Result<()> {
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

    fn schedule_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Tägliche Uhrzeit (HH:MM):");
            ui.text_edit_singleline(&mut self.cfg.schedule_time);
        });
        ui.horizontal(|ui| {
            if ui.button("Zeitplan aktivieren").clicked() {
                self.status = match self.cfg.save().and_then(|_| schedule::install(&self.cfg.schedule_time)) {
                    Ok(()) => "Zeitplan aktiv.".into(),
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
        ui.label(&self.status);
        #[cfg(unix)]
        {
            ui.separator();
            ui.label("Geplante Läufe können keinen pkexec-Dialog bestätigen. Trage daher diese Zeile in /etc/fstab ein (als root); danach mountet die App ohne Rückfrage:");
            let mut line = smb::fstab_line(&self.cfg).unwrap_or_default();
            ui.add(egui::TextEdit::multiline(&mut line).desired_rows(3).desired_width(f32::INFINITY));
            if ui.button("Credentials-Datei anlegen (SMB-Passwort aus Keyring)").clicked() {
                self.status = match secrets::get(Secret::Smb).and_then(|p| p.ok_or_else(|| anyhow::anyhow!("Kein SMB-Passwort gespeichert"))).and_then(|p| smb::write_credentials(&self.cfg, &p)) {
                    Ok(()) => "Credentials-Datei (0600) angelegt.".into(),
                    Err(e) => format!("FEHLER: {e:#}"),
                };
            }
        }
        #[cfg(windows)]
        ui.label("Der Taskplaner-Task läuft unter deinem Benutzer und nutzt die Freigabe direkt per UNC-Pfad.");
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = smb::unmount(&self.cfg);
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll();
        ui.horizontal(|ui| {
            for (t, name) in [(Tab::Backup, "Backup"), (Tab::Snapshots, "Snapshots"), (Tab::Settings, "Einstellungen"), (Tab::Schedule, "Zeitplan")] {
                if ui.selectable_label(self.tab == t, name).clicked() {
                    self.tab = t;
                    self.status.clear();
                }
            }
        });
        ui.separator();
        match self.tab {
            Tab::Backup => self.backup_tab(ui),
            Tab::Snapshots => self.snapshots_tab(ui),
            Tab::Settings => self.settings_tab(ui),
            Tab::Schedule => self.schedule_tab(ui),
        }
    }
}
