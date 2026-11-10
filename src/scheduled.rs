//! Zeitplan-Lauf (`--scheduled`): entscheidet selbst, ob gesichert wird. Der Zeitplan startet die
//! App oft (Uhrzeiten, Anmeldung, Entsperren, Aufwachen, Netzwerk verbunden, alle 10 Minuten); gesichert
//! wird nur, wenn seit dem letzten geplanten Termin noch kein Backup gelang, keine Pause aktiv ist und das
//! NAS erreichbar ist. In allen anderen Fällen endet der Lauf, ohne `last-run.log` zu überschreiben; die
//! Entscheidung steht dann nur in `schedule.log`.
use crate::config::{self, Config};
use crate::schedule;
use anyhow::Result;
use chrono::NaiveDateTime;

#[derive(Debug, PartialEq)]
pub enum Decision {
    Run,
    Skip(&'static str),
    /// Nach einem Fehlschlag: frühestens zu diesem Zeitpunkt neu versuchen
    Wait(NaiveDateTime),
}

/// Wartezeit in Minuten nach dem `count`-ten Fehlschlag in Folge. Kurze Störungen (Netz nach dem Aufwachen noch
/// nicht bereit) sind so nach wenigen Minuten überwunden; bei einem dauerhaften Fehler gibt es trotzdem nicht
/// alle paar Minuten einen Versuch samt Verlaufseintrag.
pub fn retry_delay_minutes(count: u32) -> i64 {
    match count {
        0 => 0,
        1 => 5,
        2 => 15,
        3 => 60,
        _ => 120,
    }
}

/// Reine Entscheidung ohne Seiteneffekte: Pause, Fälligkeit, Wartezeit nach Fehler.
/// `failures`: fehlgeschlagene Zeitplan-Läufe in Folge und Zeitpunkt des letzten.
pub fn decide(now: NaiveDateTime, cfg: &Config, last_success: Option<NaiveDateTime>, pause: Option<NaiveDateTime>, failures: Option<(u32, NaiveDateTime)>) -> Result<Decision> {
    if config::pause_active(pause, now) {
        return Ok(Decision::Skip("Zeitplan ist pausiert"));
    }
    let slot = schedule::last_slot(now, &cfg.schedule_times, &cfg.schedule_days)?;
    if !schedule::is_due(last_success, slot) {
        return Ok(Decision::Skip("Kein Backup fällig"));
    }
    // Nur ein Fehler nach dem fälligen Termin zählt (ein älterer betraf einen früheren Termin)
    if let Some((count, failed)) = failures.filter(|(_, f)| slot.is_some_and(|s| *f >= s)) {
        let next = failed + chrono::Duration::minutes(retry_delay_minutes(count));
        if now < next {
            return Ok(Decision::Wait(next));
        }
    }
    Ok(Decision::Run)
}

/// Ganze Tage seit dem letzten Erfolg, wenn die Warnschwelle erreicht ist.
pub fn overdue_days(now: NaiveDateTime, last_success: Option<NaiveDateTime>, warn_after_days: u32) -> Option<i64> {
    let days = now.signed_duration_since(last_success?).num_days();
    (warn_after_days > 0 && days >= warn_after_days as i64).then_some(days)
}

/// Die Meldung höchstens einmal je 24 Stunden.
pub fn notice_allowed(last_notice: Option<NaiveDateTime>, now: NaiveDateTime) -> bool {
    last_notice.is_none_or(|t| now.signed_duration_since(t).num_hours() >= 24)
}

/// Meldet „Backup überfällig“, wenn die Warnschwelle erreicht ist (höchstens einmal je 24 Stunden). Gelang noch
/// nie ein Backup, zählen die Tage ab dem Einrichten des Zeitplans. Liefert `true`, wenn gemeldet wurde.
pub fn warn_if_overdue(now: NaiveDateTime, cfg: &Config) -> bool {
    let last_success = config::read_last_success();
    // Nur wenn auch ein Termin offen ist: Bei einem Zeitplan „nur montags“ ist am Freitag nichts überfällig
    if !schedule::last_slot(now, &cfg.schedule_times, &cfg.schedule_days).is_ok_and(|slot| schedule::is_due(last_success, slot)) {
        return false;
    }
    let since = last_success.or_else(schedule::activated_at);
    match overdue_days(now, since, cfg.warn_after_days).filter(|_| notice_allowed(config::read_overdue_notice(), now)) {
        Some(days) => {
            crate::notify::backup_overdue(days);
            config::write_overdue_notice();
            true
        }
        None => false,
    }
}

/// Wartet kurz auf das NAS (Netz nach dem Aufwachen); `false`, wenn es nicht erreichbar wird.
fn wait_reachable(host: &str, attempts: u32, wait: std::time::Duration) -> bool {
    for i in 1..=attempts {
        if crate::smb::host_reachable(host) {
            return true;
        }
        if i < attempts {
            std::thread::sleep(wait);
        }
    }
    false
}

/// Was täte ein Zeitplan-Lauf jetzt? Für „Zeitplan prüfen“ im Fenster; startet kein Backup.
pub fn dry_run(cfg: &Config) -> String {
    if !cfg.is_ready() {
        return "Warnung: Nicht eingerichtet (NAS-Adresse, Freigabe und mindestens ein Quellordner sind nötig).".into();
    }
    let now = chrono::Local::now().naive_local();
    match decide(now, cfg, config::read_last_success(), config::read_pause(), config::read_schedule_failures()) {
        Err(e) => format!("FEHLER: {e:#}"),
        Ok(Decision::Skip(why)) => format!("{why} – ein Zeitplan-Lauf würde jetzt nichts tun."),
        Ok(Decision::Wait(next)) => format!("Warnung: Backup fällig, aber der letzte Versuch schlug fehl – nächster Versuch ab {}.", next.format("%H:%M")),
        Ok(Decision::Run) if crate::smb::host_reachable(&cfg.host) => "Backup fällig und NAS erreichbar – der nächste Zeitplan-Lauf sichert.".into(),
        Ok(Decision::Run) => format!("Warnung: Backup fällig, aber NAS {} nicht erreichbar (Port 445) – der Zeitplan versucht es weiter.", cfg.host),
    }
}

/// Hält eine Entscheidung fest: auf der Konsole (Linux: Journal) und in `schedule.log` für die Anzeige im Fenster.
pub fn note(msg: &str) {
    println!("{msg}");
    config::append_schedule_log(msg);
}

/// Einstiegspunkt für `--scheduled`.
pub fn run() -> Result<()> {
    let cfg = Config::load()?;
    if !cfg.is_ready() {
        note("Nicht eingerichtet – übersprungen");
        return Ok(());
    }
    let now = chrono::Local::now().naive_local();
    match decide(now, &cfg, config::read_last_success(), config::read_pause(), config::read_schedule_failures())? {
        Decision::Run => {}
        Decision::Skip(why) => {
            note(why);
            return Ok(());
        }
        Decision::Wait(next) => {
            note(&format!("Letzter Versuch fehlgeschlagen – nächster Versuch ab {}", next.format("%H:%M")));
            warn_if_overdue(now, &cfg);
            return Ok(());
        }
    }
    if !wait_reachable(&cfg.host, 3, std::time::Duration::from_secs(20)) {
        note(&format!("Backup fällig, aber NAS {} nicht erreichbar – später erneut", cfg.host));
        warn_if_overdue(now, &cfg);
        return Ok(());
    }
    // Kurz nach dem Anmelden oder Aufwachen läuft der Schlüsselbund oft noch nicht: bis zu 4 Minuten warten, statt
    // gleich mit einem Fehler zu enden. Bleibt er aus, meldet der Lauf den Fehler wie bisher.
    if !wait_store(8, std::time::Duration::from_secs(30)) {
        note("Schlüsselbund nicht erreichbar");
    }
    note("Backup gestartet");
    let result = crate::run_headless(true);
    if result.is_err() {
        warn_if_overdue(chrono::Local::now().naive_local(), &cfg);
    }
    result
}

/// Wartet auf den System-Schlüsselbund; `false`, wenn er nicht erreichbar wird.
fn wait_store(attempts: u32, wait: std::time::Duration) -> bool {
    for i in 1..=attempts {
        if crate::secrets::store_ready() {
            return true;
        }
        if i < attempts {
            println!("Schlüsselbund noch nicht bereit – neuer Versuch in {} s ({i}/{attempts}).", wait.as_secs());
            std::thread::sleep(wait);
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDateTime::new(NaiveDate::from_ymd_opt(2026, 11, day).unwrap(), NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }

    fn cfg() -> Config {
        let mut c = Config::default();
        c.schedule_times = vec!["02:00".into(), "14:00".into()];
        c
    }

    #[test]
    fn run_when_slot_missed() {
        // Laptop war aus: letzter Erfolg vor drei Tagen, jetzt Mittwoch 16:00
        assert_eq!(decide(at(4, 16, 0), &cfg(), Some(at(1, 14, 5)), None, None).unwrap(), Decision::Run);
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, None, None).unwrap(), Decision::Run);
    }

    #[test]
    fn skip_when_fresh_or_paused() {
        assert!(matches!(decide(at(4, 16, 0), &cfg(), Some(at(4, 14, 10)), None, None).unwrap(), Decision::Skip(_)));
        // Pause hat Vorrang, abgelaufene Pause nicht
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, Some(at(5, 8, 0)), None).unwrap(), Decision::Skip("Zeitplan ist pausiert"));
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, Some(at(4, 8, 0)), None).unwrap(), Decision::Run);
    }

    #[test]
    fn waits_after_failure() {
        // Erster Fehler um 15:30 nach dem Termin 14:00: 5 Minuten warten
        assert_eq!(decide(at(4, 15, 32), &cfg(), None, None, Some((1, at(4, 15, 30)))).unwrap(), Decision::Wait(at(4, 15, 35)));
        assert_eq!(decide(at(4, 15, 35), &cfg(), None, None, Some((1, at(4, 15, 30)))).unwrap(), Decision::Run);
        // Vierter Fehler in Folge: 2 Stunden
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, None, Some((4, at(4, 15, 30)))).unwrap(), Decision::Wait(at(4, 17, 30)));
        assert_eq!(decide(at(4, 17, 30), &cfg(), None, None, Some((4, at(4, 15, 30)))).unwrap(), Decision::Run);
        // Fehler vor dem fälligen Termin (früherer Termin) bremst nicht
        assert_eq!(decide(at(4, 14, 5), &cfg(), None, None, Some((9, at(4, 13, 50)))).unwrap(), Decision::Run);
        // Pause und „nicht fällig“ gehen vor
        assert!(matches!(decide(at(4, 15, 32), &cfg(), Some(at(4, 14, 10)), None, Some((1, at(4, 15, 30)))).unwrap(), Decision::Skip(_)));
    }

    #[test]
    fn retry_delay_grows() {
        assert_eq!([0, 1, 2, 3, 4, 50].map(retry_delay_minutes), [0, 5, 15, 60, 120, 120]);
    }

    #[test]
    fn overdue_threshold() {
        assert_eq!(overdue_days(at(5, 12, 0), Some(at(2, 12, 0)), 3), Some(3));
        assert_eq!(overdue_days(at(5, 11, 0), Some(at(2, 12, 0)), 3), None);
        assert_eq!(overdue_days(at(20, 12, 0), Some(at(2, 12, 0)), 0), None);
        assert_eq!(overdue_days(at(20, 12, 0), None, 3), None);
    }

    #[test]
    fn notice_once_a_day() {
        assert!(notice_allowed(None, at(5, 12, 0)));
        assert!(!notice_allowed(Some(at(5, 1, 0)), at(5, 12, 0)));
        assert!(notice_allowed(Some(at(4, 11, 0)), at(5, 12, 0)));
    }
}
