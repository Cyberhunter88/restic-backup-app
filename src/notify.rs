//! Desktop-Benachrichtigung nach einem Lauf – nur bei Warnung oder Fehler.

/// Titel und Text der Meldung; `None` bei einem sauberen Erfolg.
pub fn message(warnings: bool, error: Option<&str>) -> Option<(String, String)> {
    match (error, warnings) {
        (Some(e), _) => {
            // Mehrzeilige Fehler auf die erste Zeile kürzen
            let first = e.lines().next().unwrap_or(e);
            Some(("Backup fehlgeschlagen".into(), first.chars().take(200).collect()))
        }
        (None, true) => Some(("Backup mit Warnungen".into(), "Einige Dateien oder Prüfungen hatten Probleme – Details im Protokoll.".into())),
        (None, false) => None,
    }
}

/// Sendet die Benachrichtigung; Fehler dabei (z. B. kein Benachrichtigungsdienst) werden ignoriert.
pub fn backup_finished(warnings: bool, error: Option<&str>) {
    if let Some((title, body)) = message(warnings, error) {
        let _ = notify_rust::Notification::new().appname("restic Backup").summary(&title).body(&body).show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_bei_problemen() {
        assert!(message(false, None).is_none());
        assert_eq!(message(true, None).unwrap().0, "Backup mit Warnungen");
        let (t, b) = message(false, Some("erste\nzweite")).unwrap();
        assert_eq!(t, "Backup fehlgeschlagen");
        assert_eq!(b, "erste");
    }
}
