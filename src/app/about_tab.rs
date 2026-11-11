use super::*;

impl App {
    pub(super) fn about_tab(&mut self, ui: &mut egui::Ui) {
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
            ui.checkbox(&mut self.cfg.notify_success, "Auch nach erfolgreichem Backup eine Desktop-Meldung senden");
            ui::hint(ui, if self.has_tray() {
                "Das X des Fensters lässt die App im Hintergrund weiterlaufen (Tray-Icon). Ganz beenden: „Beenden“ in der Seitenleiste oder im Tray-Menü."
            } else {
                "Das X des Fensters lässt die App im Hintergrund weiterlaufen. Kein Tray-Icon verfügbar: Das Fenster kommt zurück, wenn die App erneut gestartet wird. Ganz beenden: „Beenden“ in der Seitenleiste."
            });
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
}
