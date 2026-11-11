use super::*;

impl App {
    pub(super) fn schedule_tab(&mut self, ui: &mut egui::Ui) {
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
                        // Das Einrichten selbst (PowerShell bzw. systemctl) läuft im Hintergrund, damit das Fenster
                        // nicht einfriert; `Msg::SchedDone` bringt den Statustext und lädt die Diagnose neu
                        match self.cfg.validate().and_then(|_| self.cfg.save()) {
                            Err(e) => self.status = format!("FEHLER: {e:#}"),
                            Ok(()) => {
                                let (times, days, opts, summary) = (self.cfg.schedule_times.clone(), self.cfg.schedule_days, self.sched_opts(), self.schedule_summary());
                                self.status = "Richte Zeitplan ein …".into();
                                self.spawn(ui.ctx(), move |tx| {
                                    let msg = match schedule::install(&times, &days, opts) {
                                        Ok(()) => format!("Zeitplan aktiv: {summary}."),
                                        Err(e) => format!("FEHLER: {e:#}"),
                                    };
                                    // Ohne fstab-Eintrag kann ein geplanter Lauf die Freigabe nicht einbinden (kein pkexec-Dialog)
                                    #[cfg(unix)]
                                    let msg = if !msg.starts_with("FEHLER") && !smb::scheduled_mount_ready() {
                                        "Warnung: Zeitplan aktiv, aber der fstab-Eintrag fehlt. Geplante Läufe können die Freigabe so nicht einbinden – siehe „Automatisches Einbinden (Linux)“ unten.".to_string()
                                    } else {
                                        msg
                                    };
                                    let _ = tx.send(Msg::SchedDone(msg));
                                });
                            }
                        }
                    }
                    if ui.button("Zeitplan entfernen").clicked() {
                        self.status = "Entferne Zeitplan …".into();
                        self.spawn(ui.ctx(), move |tx| {
                            let msg = match schedule::remove() {
                                Ok(()) => "Zeitplan entfernt.".to_string(),
                                Err(e) => format!("FEHLER: {e:#}"),
                            };
                            let _ = tx.send(Msg::SchedDone(msg));
                        });
                    }
                });
                ui::status(ui, &self.status);
                ui::hint_long(
                    ui,
                    "hint_catchup",
                    "Änderungen gelten erst nach „Zeitplan aktivieren“. Verpasste Läufe werden nachgeholt.",
                    "Der Zeitplan startet das Programm zu den Uhrzeiten, bei der Anmeldung, beim Entsperren und alle 10 Minuten (Windows zusätzlich beim Aufwachen und sobald ein Netzwerk verbunden ist); gesichert wird nur, wenn seit dem letzten geplanten Termin noch kein Backup gelang. War der Rechner aus, wird also einmal nachgeholt (auch nach Tagen). Ist das NAS nicht erreichbar (z. B. unterwegs), wird der Lauf übersprungen und später erneut versucht – ohne Fehler im Verlauf. Schlägt ein Lauf fehl, versucht es der Zeitplan nach 5 Minuten erneut, danach nach 15 Minuten, einer Stunde und dann alle zwei Stunden; eine Desktop-Meldung kommt erst beim zweiten Fehlschlag in Folge. Ein abgebrochenes Backup zählt nicht als Fehlschlag. Nach einer Programm-Aktualisierung wird der Zeitplan beim Start der App automatisch erneuert.",
                );
                #[cfg(windows)]
                ui::hint(ui, "Der Taskplaner-Task läuft unter deinem Benutzer und nutzt die Freigabe direkt per UNC-Pfad.");
            });
            ui::card(ui, Some("Diagnose"), |ui| {
                let now = chrono::Local::now().naive_local();
                egui::Grid::new("sched_diag").num_columns(2).spacing([18.0, 6.0]).show(ui, |ui| {
                    ui.label("Zeitplan im System");
                    match &self.sched_status {
                        None => ui.label(RichText::new("wird abgefragt …").color(MUTED)),
                        Some(s) if !s.installed => ui.label(RichText::new("nicht eingerichtet").color(WARN)),
                        Some(s) if !s.active => ui.label(RichText::new("eingerichtet, aber deaktiviert").color(ERR)),
                        Some(_) => ui.label(RichText::new("aktiv").color(OK)),
                    };
                    ui.end_row();
                    if let Some(s) = self.sched_status.as_ref().filter(|s| s.installed) {
                        if let Some(exe) = &s.exe {
                            ui.label("Programmdatei");
                            if s.exe_exists {
                                ui.label(RichText::new(exe).color(MUTED));
                            } else {
                                ui.label(RichText::new(format!("{exe} – fehlt! „Zeitplan aktivieren“ richtet ihn neu ein.")).color(ERR));
                            }
                            ui.end_row();
                        }
                        ui.label("Letzter Start");
                        ui.label(match (&s.last_run, &s.result) {
                            (Some(t), Some(r)) => format!("{t} ({r})"),
                            (Some(t), None) => t.clone(),
                            (None, Some(r)) => r.clone(),
                            (None, None) => "noch keiner".into(),
                        });
                        ui.end_row();
                    }
                    ui.label("Letztes Backup");
                    ui.label(self.sched_last_success.map_or("noch keins".to_string(), |t| t.format("%d.%m.%Y %H:%M").to_string()));
                    ui.end_row();
                    ui.label("Nächster Termin");
                    ui.label(match schedule::next_slot(now, &self.cfg.schedule_times, &self.cfg.schedule_days) {
                        Ok(Some(t)) => t.format("%d.%m.%Y %H:%M").to_string(),
                        _ => "keiner (Uhrzeit oder Wochentag fehlt)".into(),
                    });
                    ui.end_row();
                    if let Some((count, failed)) = self.sched_failures {
                        let next = failed + chrono::Duration::minutes(crate::scheduled::retry_delay_minutes(count));
                        ui.label("Fehlversuche");
                        ui.label(RichText::new(format!("{count} in Folge, zuletzt {} – nächster Versuch ab {}", failed.format("%d.%m. %H:%M"), next.format("%H:%M"))).color(WARN));
                        ui.end_row();
                    }
                    if !self.sched_mount_ready {
                        ui.label("Einbinden");
                        ui.label(RichText::new("fstab-Eintrag fehlt – geplante Läufe können die Freigabe nicht einbinden").color(ERR));
                        ui.end_row();
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("Zeitplan prüfen").on_hover_text("Zeigt, was ein Zeitplan-Lauf jetzt täte. Startet kein Backup.").clicked() {
                        self.check_schedule(ui.ctx());
                    }
                    if ui.button("Aktualisieren").clicked() {
                        self.load_schedule_status(ui.ctx());
                    }
                });
                ui::status(ui, &self.sched_check);
                ui.separator();
                ui.label(RichText::new("Letzte Entscheidungen des Zeitplans").color(MUTED).size(13.0));
                if self.sched_log.is_empty() {
                    ui::hint(ui, "Noch keine Einträge.");
                }
                for (time, msg) in &self.sched_log {
                    // Zeitstempel „JJJJ-MM-TT HH:MM:SS“ kürzen auf „TT.MM. HH:MM“
                    let short = chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%d %H:%M:%S").map_or(time.clone(), |t| t.format("%d.%m. %H:%M").to_string());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(short).monospace().color(MUTED).size(13.0));
                        ui.label(RichText::new(msg).size(13.0));
                    });
                }
            });
            ui::card(ui, Some("Laptop und Pause"), |ui| {
                ui.checkbox(&mut self.cfg.only_on_ac, "Nur am Netzteil sichern");
                #[cfg(windows)]
                ui.checkbox(&mut self.cfg.wake_to_run, "Rechner zu den Uhrzeiten aus dem Standby wecken").on_hover_text("Windows weckt den Rechner nur, wenn in den Energieoptionen „Zeitgeber zur Aktivierung zulassen“ eingeschaltet ist.");
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
}
