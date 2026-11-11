use super::*;

impl App {
    pub(super) fn backup_tab(&mut self, ui: &mut egui::Ui) {
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
                            ui.horizontal(|ui| {
                                if head.contains("ERFOLGREICH") {
                                    ui::badge(ui, "Erfolgreich", OK);
                                } else if head.contains("ABGEBROCHEN") {
                                    ui::badge(ui, "Abgebrochen", WARN);
                                } else {
                                    ui::badge(ui, "Fehler", ERR);
                                }
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
                    ui.add_enabled_ui(!self.busy && self.external_run.is_none(), |ui| {
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
                                let low = job::low_space_warning(Some(f), self.cfg.warn_free_gb).is_some();
                                let text = RichText::new(format!("Frei auf dem NAS: {}{}", ui::human_bytes(f), if low { " – wird knapp" } else { "" }));
                                ui.label(if low { text.color(WARN) } else { text });
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
                    if self.progress_text.is_empty() {
                        ui.add(egui::ProgressBar::new(self.progress).show_percentage().desired_width(ui.available_width() - 110.0));
                    } else {
                        ui.label(&self.progress_text);
                        ui.add_space((ui.available_width() - 110.0).max(0.0));
                    }
                    if ui.button("Abbrechen").clicked() {
                        self.cancel.store(true, Ordering::Relaxed);
                    }
                });
            });
        }

        // Backup in einem anderen Prozess (Zeitplan): anzeigen und abbrechen lassen
        if let Some(run) = self.external_run.clone() {
            let title = match run.started {
                Some(t) => format!("Zeitplan-Backup läuft (seit {})", t.format("%H:%M")),
                None => "Zeitplan-Backup läuft".to_string(),
            };
            ui::card(ui, Some(&title), |ui| {
                if !run.text.is_empty() {
                    ui::hint(ui, &run.text);
                }
                ui.horizontal(|ui| {
                    ui.add(egui::ProgressBar::new(run.percent).show_percentage().desired_width(ui.available_width() - 110.0));
                    if ui.add_enabled(!self.external_cancel, egui::Button::new("Abbrechen")).clicked() {
                        crate::config::request_cancel();
                        self.external_cancel = true;
                    }
                });
                if self.external_cancel {
                    ui::hint(ui, "Abbruch angefordert – der Lauf endet nach dem aktuellen Schritt.");
                }
            });
        }

        // Der neueste Lauf steht schon oben unter „Letzter Lauf“
        if self.history.len() > 1 {
            ui::card(ui, Some("Frühere Läufe"), |ui| {
                egui::Grid::new("history").num_columns(3).striped(true).spacing([16.0, 6.0]).show(ui, |ui| {
                    for h in self.history.iter().skip(1) {
                        ui.label(RichText::new(&h.time).color(MUTED));
                        match h.result {
                            crate::config::RunResult::Ok => ui::badge(ui, "OK", OK),
                            crate::config::RunResult::Cancelled => ui::badge(ui, "Abgebrochen", WARN),
                            crate::config::RunResult::Failed => ui::badge(ui, "Fehler", ERR),
                        }
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
}
