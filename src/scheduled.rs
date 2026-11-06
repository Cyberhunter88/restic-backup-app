//! Zeitplan-Lauf (`--scheduled`): entscheidet selbst, ob gesichert wird. Der Zeitplan startet die
//! App oft (Uhrzeiten, Anmeldung, Entsperren, alle 30 Minuten); gesichert wird nur, wenn seit dem
//! letzten geplanten Termin noch kein Backup gelang, keine Pause aktiv ist und das NAS erreichbar
//! ist. In allen anderen Fällen endet der Lauf still, ohne `last-run.log` zu überschreiben.
use crate::config::{self, Config};
use crate::schedule;
use anyhow::Result;
use chrono::NaiveDateTime;

#[derive(Debug, PartialEq)]
pub enum Decision {
    Run,
    Skip(&'static str),
}

/// Reine Entscheidung ohne Seiteneffekte: Pause, Fälligkeit.
pub fn decide(now: NaiveDateTime, cfg: &Config, last_success: Option<NaiveDateTime>, pause: Option<NaiveDateTime>) -> Result<Decision> {
    if config::pause_active(pause, now) {
        return Ok(Decision::Skip("Zeitplan ist pausiert"));
    }
    let slot = schedule::last_slot(now, &cfg.schedule_times, &cfg.schedule_days)?;
    Ok(if schedule::is_due(last_success, slot) { Decision::Run } else { Decision::Skip("Kein Backup fällig") })
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

/// Einstiegspunkt für `--scheduled`.
pub fn run() -> Result<()> {
    let cfg = Config::load()?;
    if !cfg.is_ready() {
        println!("Nicht eingerichtet – übersprungen.");
        return Ok(());
    }
    let now = chrono::Local::now().naive_local();
    let last_success = config::read_last_success();
    if let Decision::Skip(why) = decide(now, &cfg, last_success, config::read_pause())? {
        println!("{why} – übersprungen.");
        return Ok(());
    }
    if !wait_reachable(&cfg.host, 3, std::time::Duration::from_secs(20)) {
        println!("NAS {} nicht erreichbar – übersprungen.", cfg.host);
        if let Some(days) = overdue_days(now, last_success, cfg.warn_after_days).filter(|_| notice_allowed(config::read_overdue_notice(), now)) {
            crate::notify::backup_overdue(days);
            config::write_overdue_notice();
        }
        return Ok(());
    }
    crate::run_headless(true)
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
        assert_eq!(decide(at(4, 16, 0), &cfg(), Some(at(1, 14, 5)), None).unwrap(), Decision::Run);
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, None).unwrap(), Decision::Run);
    }

    #[test]
    fn skip_when_fresh_or_paused() {
        assert!(matches!(decide(at(4, 16, 0), &cfg(), Some(at(4, 14, 10)), None).unwrap(), Decision::Skip(_)));
        // Pause hat Vorrang, abgelaufene Pause nicht
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, Some(at(5, 8, 0))).unwrap(), Decision::Skip("Zeitplan ist pausiert"));
        assert_eq!(decide(at(4, 16, 0), &cfg(), None, Some(at(4, 8, 0))).unwrap(), Decision::Run);
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
