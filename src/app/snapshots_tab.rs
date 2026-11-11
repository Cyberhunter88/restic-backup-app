use super::*;

impl App {
    pub(super) fn snapshots_tab(&mut self, ui: &mut egui::Ui) {
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
                    let cancel = self.cancel.clone();
                    self.spawn(&ctx, move |tx| {
                        let r = job::open(&cfg).and_then(|r| r.check(&cancel));
                        // Eine abgebrochene Prüfung ist kein Ergebnis
                        if !cancel.load(Ordering::Relaxed) {
                            crate::config::write_last_check(r.is_ok());
                        }
                        let _ = tx.send(Msg::Info(r.map(|_| "Repository ist in Ordnung.".to_string()).map_err(|e| format!("{e:#}"))));
                    });
                    self.cancellable = true;
                }
                let all = self.unlock_all;
                if ui
                    .add_enabled(!self.busy, egui::Button::new("🔓  Sperren entfernen"))
                    .on_hover_text(if all { "restic unlock --remove-all" } else { "restic unlock (nur veraltete Sperren)" })
                    .clicked()
                {
                    let cfg = self.cfg.clone();
                    self.status = "Entferne Sperren …".into();
                    self.spawn(&ctx, move |tx| {
                        let r = job::unlock_repo(&cfg, all);
                        let _ = tx.send(Msg::Info(r.map_err(|e| format!("{e:#}"))));
                    });
                }
                ui.checkbox(&mut self.unlock_all, "auch aktive")
                    .on_hover_text("Entfernt ALLE Sperren, auch die eines gerade laufenden Backups (z. B. von einem anderen Rechner). Nur nutzen, wenn sicher nichts läuft.");
                ui::status(ui, &self.status);
                // Wiederherstellung, Prüfung und Suche lassen sich abbrechen
                if self.busy && self.cancellable {
                    if let Some(p) = self.task_progress {
                        ui.add(egui::ProgressBar::new(p).show_percentage().desired_width(160.0));
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                }
            });
            ui.add_enabled_ui(!self.restore_in_place, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Wiederherstellen nach:");
                    ui.add(egui::TextEdit::singleline(&mut self.restore_target).desired_width(ui.available_width() - 110.0).hint_text("Zielordner wählen"));
                    if ui.button("Wählen …").clicked() {
                        if let Some(p) = rfd::FileDialog::new().pick_folder() {
                            self.restore_target = p.to_string_lossy().into_owned();
                        }
                    }
                });
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(&mut self.restore_in_place, "Am Originalort wiederherstellen").on_hover_text("Schreibt die Dateien dorthin zurück, wo sie gesichert wurden (braucht restic 0.17 oder neuer). Vor dem Start wird nachgefragt.");
                ui.label("Vorhandene Dateien");
                egui::ComboBox::from_id_salt("restore_overwrite").selected_text(self.restore_overwrite.label()).show_ui(ui, |ui| {
                    for o in crate::restore::Overwrite::ALL {
                        ui.selectable_value(&mut self.restore_overwrite, o, o.label());
                    }
                });
                ui.checkbox(&mut self.restore_verify, "Danach prüfen").on_hover_text("Liest die wiederhergestellten Dateien noch einmal und vergleicht sie mit dem Snapshot (restic restore --verify).");
            });
            ui::hint(ui, "Die Wahl für vorhandene Dateien wirkt ab restic 0.17; ältere Versionen überschreiben immer.");
            // Sicherheitsabfrage vor dem Zurückschreiben an den Originalort
            if let Some((_, include)) = self.confirm_restore.clone() {
                ui.separator();
                let what = include.unwrap_or_else(|| "den ganzen Snapshot".into());
                ui.label(RichText::new(format!("{what} wirklich an den Originalort zurückschreiben? Vorhandene Dateien: {}.", self.restore_overwrite.label())).color(WARN));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("Ja, wiederherstellen")).clicked() {
                        if let Some((id, include)) = self.confirm_restore.take() {
                            self.restore(&ctx, id, include);
                        }
                    }
                    if ui.button("Nicht wiederherstellen").clicked() {
                        self.confirm_restore = None;
                    }
                });
            }
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
        // Einmal je Frame filtern (statt je Snapshot zweimal: für die Anzahl und für die Liste)
        let visible: Vec<usize> = (0..self.snapshots.len()).filter(|&i| ui::snapshot_matches(&self.snapshots[i], &self.snap_filter, self.snap_folder.as_deref(), self.snap_range, now)).collect();
        let shown = visible.len();
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
                        for s in visible.iter().map(|&i| &self.snapshots[i]) {
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
            self.found = None;
            self.load_dir(&ctx);
        }
        let Some(id) = self.selected.clone() else {
            ui::hint(ui, "Wähle einen Snapshot, um seine Dateien zu durchsuchen.");
            return;
        };

        let mut enter = None;
        let mut restore = None;
        let mut search = false;
        let mut clear_search = false;
        let can_restore = !self.busy && (self.restore_in_place || !self.restore_target.is_empty());
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
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.find_term).hint_text("🔍 Datei im Snapshot suchen (Name, * und ? möglich)").desired_width(340.0));
                let enter_pressed = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.add_enabled(!self.busy && !self.find_term.trim().is_empty(), egui::Button::new("Suchen")).clicked() || enter_pressed) && !self.busy && !self.find_term.trim().is_empty() {
                    search = true;
                }
                if self.found.is_some() && ui.button("✖ Suche beenden").clicked() {
                    clear_search = true;
                }
            });
            if let Some(f) = &self.found {
                ui::hint(ui, &format!("{} Treffer für „{}“ – Pfade beginnen bei Windows mit dem Laufwerksbuchstaben.", f.len(), self.find_term.trim()));
            }
            let searching = self.found.is_some();
            let list: &[Node] = self.found.as_deref().unwrap_or(&self.nodes);
            egui::ScrollArea::vertical().id_salt("nodes").auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("node_grid").num_columns(3).striped(true).min_col_width(90.0).show(ui, |ui| {
                    for n in list {
                        if searching {
                            // Treffer: voller Pfad, Ordner lassen sich öffnen
                            if n.kind == "dir" {
                                if ui.add_enabled(!self.busy, egui::Button::new(format!("📁 {}", n.path)).frame(false)).clicked() {
                                    enter = Some(n.path.clone());
                                    clear_search = true;
                                }
                                ui.label("");
                            } else {
                                ui.label(&n.path);
                                ui.label(RichText::new(ui::human_bytes(n.size)).color(MUTED));
                            }
                        } else if n.kind == "dir" {
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
        if clear_search {
            self.found = None;
        }
        if search {
            let (cfg, term, snap) = (self.cfg.clone(), self.find_term.clone(), id.clone());
            self.status = "Suche …".into();
            let cancel = self.cancel.clone();
            self.spawn(&ctx, move |tx| {
                let r = job::open(&cfg).and_then(|r| r.find(&snap, &term, FIND_LIMIT, &cancel));
                let _ = tx.send(Msg::Found(r.map_err(|e| format!("{e:#}"))));
            });
            self.cancellable = true;
        }
        if let Some(d) = enter {
            self.dir = d;
            self.load_dir(&ctx);
        }
        if let Some((include, _)) = restore {
            if self.restore_in_place {
                // Erst nach der Sicherheitsabfrage (oben im Tab)
                self.confirm_restore = Some((id.clone(), include));
            } else {
                self.restore(&ctx, id.clone(), include);
            }
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

    /// Stellt einen Pfad (`include`) oder den ganzen Snapshot wieder her: in den Zielordner oder, mit
    /// `restore_in_place`, an den Originalort (unter Windows je Laufwerk ein restic-Aufruf).
    fn restore(&mut self, ctx: &egui::Context, id: String, include: Option<String>) {
        use crate::restore::{RestoreJob, RestoreOptions};
        let (in_place, overwrite, verify) = (self.restore_in_place, self.restore_overwrite, self.restore_verify);
        self.restored_to = None;
        let jobs = if in_place {
            let paths = self.snapshots.iter().find(|s| s.id == id).map(|s| s.paths.clone()).unwrap_or_default();
            match crate::restore::in_place_jobs(&id, include.as_deref(), &paths, cfg!(windows)) {
                Ok(j) => j,
                Err(e) => {
                    self.status = format!("FEHLER: {e:#}");
                    return;
                }
            }
        } else {
            vec![RestoreJob { snapshot: id, target: self.restore_target.clone(), include }]
        };
        // Leer = Originalort (dann gibt es keinen einzelnen Ordner zum Öffnen)
        let shown_target = if in_place { String::new() } else { self.restore_target.clone() };
        self.status = "Stelle wieder her …".into();
        let (cfg, cancel) = (self.cfg.clone(), self.cancel.clone());
        self.spawn(ctx, move |tx| {
            let r = job::open(&cfg).and_then(|r| {
                for j in &jobs {
                    let opts = RestoreOptions { snapshot: &j.snapshot, target: &j.target, include: j.include.as_deref(), overwrite, verify, in_place };
                    r.restore(&opts, &cancel, |p| drop(tx.send(Msg::RestoreProgress(p as f32))))?;
                }
                Ok(())
            });
            let _ = tx.send(Msg::Restored(r.map(|_| shown_target).map_err(|e| format!("{e:#}"))));
        });
        self.cancellable = true;
    }
}
