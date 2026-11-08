//! Desktop-Benachrichtigung nach einem Lauf – bei Warnung oder Fehler, auf Wunsch auch bei Erfolg.

/// Titel und Text der Meldung; `None` bei einem sauberen Erfolg, außer `on_success` ist gesetzt.
pub fn message(warnings: bool, error: Option<&str>, on_success: bool) -> Option<(String, String)> {
    match (error, warnings) {
        (Some(e), _) => {
            // Mehrzeilige Fehler auf die erste Zeile kürzen
            let first = e.lines().next().unwrap_or(e);
            Some(("Backup fehlgeschlagen".into(), first.chars().take(200).collect()))
        }
        (None, true) => Some(("Backup mit Warnungen".into(), "Einige Dateien oder Prüfungen hatten Probleme – Details im Protokoll.".into())),
        (None, false) if on_success => Some(("Backup erfolgreich".into(), "Das Backup wurde ohne Probleme abgeschlossen.".into())),
        (None, false) => None,
    }
}

/// Hinweis, dass seit `days` Tagen kein Backup mehr gelungen ist.
pub fn overdue_message(days: i64) -> (String, String) {
    ("Backup überfällig".into(), format!("Seit {days} Tagen ist kein Backup gelungen (NAS nicht erreichbar oder Laptop nicht am Netzteil?)."))
}

/// Meldung „überfällig“ senden; Fehler dabei werden ignoriert.
pub fn backup_overdue(days: i64) {
    let (title, body) = overdue_message(days);
    let _ = notify_rust::Notification::new().appname("restic Backup").summary(&title).body(&body).show();
}

/// Sendet die Benachrichtigung; Fehler dabei (z. B. kein Benachrichtigungsdienst) werden ignoriert.
pub fn backup_finished(warnings: bool, error: Option<&str>, on_success: bool) {
    if let Some((title, body)) = message(warnings, error, on_success) {
        let _ = notify_rust::Notification::new().appname("restic Backup").summary(&title).body(&body).show();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nur_bei_problemen() {
        assert!(message(false, None, false).is_none());
        assert_eq!(message(true, None, false).unwrap().0, "Backup mit Warnungen");
        assert_eq!(message(false, None, true).unwrap().0, "Backup erfolgreich");
        assert_eq!(message(true, None, true).unwrap().0, "Backup mit Warnungen");
        let (t, b) = message(false, Some("erste\nzweite"), true).unwrap();
        assert_eq!(t, "Backup fehlgeschlagen");
        assert_eq!(b, "erste");
        assert!(overdue_message(4).1.contains("4 Tagen"));
    }
}
