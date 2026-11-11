//! Desktop-Benachrichtigung nach einem Lauf – bei Warnung oder Fehler, auf Wunsch auch bei Erfolg.

/// Titel und Text der Meldung; `None` bei einem sauberen Erfolg, außer `on_success` ist gesetzt.
pub fn message(warnings: bool, error: Option<&str>, on_success: bool) -> Option<(String, String)> {
    match (error, warnings) {
        (Some(e), _) => {
            // Mehrzeilige Fehler auf die erste Zeile kürzen
            let first = e.lines().next().unwrap_or(e);
            Some(("Backup fehlgeschlagen".into(), first.chars().take(200).collect()))
        }
        (None, true) => Some(("Backup mit Warnungen".into(), "Einige Dateien waren nicht lesbar (z. B. von anderen Programmen gesperrt), eine Prüfung hatte Probleme oder der Platz auf dem NAS wird knapp – Details im Protokoll.".into())),
        (None, false) if on_success => Some(("Backup erfolgreich".into(), "Das Backup wurde ohne Probleme abgeschlossen.".into())),
        (None, false) => None,
    }
}

/// Hinweis, dass seit `days` Tagen kein Backup mehr gelungen ist.
pub fn overdue_message(days: i64) -> (String, String) {
    ("Backup überfällig".into(), format!("Seit {days} Tagen ist kein Backup gelungen (NAS nicht erreichbar oder Laptop nicht am Netzteil?)."))
}

/// Kennung der App für Windows-Meldungen. Ohne eigene Kennung zeigt Windows „Windows PowerShell“ als Absender.
#[cfg(windows)]
const APP_ID: &str = "Cyberhuter.ResticBackup";

/// Trägt die App-Kennung samt Anzeigenamen unter HKCU ein (ohne Adminrechte), damit Windows die Meldung
/// als „restic Backup“ anzeigt. Läuft höchstens einmal je Programmstart.
#[cfg(windows)]
fn register_app_id() {
    use std::os::windows::process::CommandExt;
    static DONE: std::sync::Once = std::sync::Once::new();
    DONE.call_once(|| {
        let key = format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}");
        let _ = std::process::Command::new("reg").args(["add", &key, "/v", "DisplayName", "/t", "REG_SZ", "/d", "restic Backup", "/f"]).creation_flags(0x0800_0000).output();
    });
}

/// Zeigt die Meldung; Fehler dabei (z. B. kein Benachrichtigungsdienst) werden ignoriert.
fn show(title: &str, body: &str) {
    let mut n = notify_rust::Notification::new();
    n.appname("restic Backup").summary(title).body(body);
    #[cfg(windows)]
    {
        register_app_id();
        n.app_id(APP_ID);
    }
    let _ = n.show();
}

/// Meldung „überfällig“ senden; Fehler dabei werden ignoriert.
pub fn backup_overdue(days: i64) {
    let (title, body) = overdue_message(days);
    show(&title, &body);
}

/// Sendet die Benachrichtigung nach einem Lauf.
pub fn backup_finished(warnings: bool, error: Option<&str>, on_success: bool) {
    if let Some((title, body)) = message(warnings, error, on_success) {
        show(&title, &body);
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
