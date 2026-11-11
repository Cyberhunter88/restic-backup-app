use super::*;

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
    pub(super) fn settings_tab(&mut self, ui: &mut egui::Ui) {
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
                    field(ui, "Rechnername", &mut self.cfg.backup_host, "leer = Name dieses Rechners (fest wählen, falls er sich ändert)");
                });
                ui.checkbox(&mut self.cfg.allow_init, "Neues Repo anlegen, falls keins existiert");
                ui.horizontal(|ui| {
                    ui.label("Warnen, wenn auf dem NAS weniger als");
                    ui.add(egui::DragValue::new(&mut self.cfg.warn_free_gb).range(0..=100_000).suffix(" GB"));
                    ui.label("frei sind (0 = aus)");
                });
                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.busy, egui::Button::new("Repo prüfen")).clicked() {
                        let cfg = self.cfg.clone();
                        self.repo_msg = "Prüfe Repository …".into();
                        self.spawn(ui.ctx(), move |tx| {
                            let _ = tx.send(Msg::RepoInfo(job::describe_repo(&cfg).map_err(|e| format!("{e:#}"))));
                        });
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
                    if ui.button(format!("Übliche Ausschlüsse ({})", crate::config::SYSTEM_NAME)).clicked() {
                        crate::config::add_excludes(&mut self.cfg.excludes, &crate::config::common_excludes().join("\n"));
                    }
                });
                ui.separator();
                ui.checkbox(&mut self.cfg.exclude_caches, "Ordner mit CACHEDIR.TAG ausschließen");
                #[cfg(unix)]
                ui.checkbox(&mut self.cfg.one_file_system, "Nicht in andere Laufwerke und Netzfreigaben wechseln")
                    .on_hover_text("Gemountete Laufwerke, USB-Platten und Netzfreigaben unterhalb eines gewählten Ordners werden nicht mitgesichert. Gewählte Ordner selbst werden immer gesichert.");
                ui.checkbox(&mut self.cfg.no_scan, "Schneller sichern: Dateien nicht vorab zählen (keine Prozentanzeige)");
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
}
