use anyhow::{Context, Result, bail};
use std::process::Command;

pub fn parse_time(t: &str) -> Result<(u32, u32)> {
    let (h, m) = t.split_once(':').context("Zeit muss HH:MM sein")?;
    let (h, m): (u32, u32) = (h.parse()?, m.parse()?);
    if h > 23 || m > 59 {
        bail!("Ungültige Uhrzeit");
    }
    Ok((h, m))
}

const DAYS_SYSTEMD: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
#[cfg_attr(unix, allow(dead_code))]
const DAYS_WINDOWS: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];

/// Sortierte, eindeutige Uhrzeiten als (Stunde, Minute).
fn parsed_times(times: &[String]) -> Result<Vec<(u32, u32)>> {
    let mut v = times.iter().map(|t| parse_time(t)).collect::<Result<Vec<_>>>()?;
    v.sort();
    v.dedup();
    if v.is_empty() {
        bail!("Keine Uhrzeit angegeben");
    }
    Ok(v)
}

/// `OnCalendar`-Zeilen für den systemd-Timer (eine pro Uhrzeit).
#[cfg_attr(windows, allow(dead_code))]
pub fn calendar_lines(times: &[String], days: &[bool; 7]) -> Result<Vec<String>> {
    if !days.iter().any(|d| *d) {
        bail!("Kein Wochentag gewählt");
    }
    let prefix = if days.iter().all(|d| *d) {
        String::new()
    } else {
        format!("{} ", DAYS_SYSTEMD.iter().zip(days).filter(|(_, on)| **on).map(|(n, _)| *n).collect::<Vec<_>>().join(","))
    };
    Ok(parsed_times(times)?.into_iter().map(|(h, m)| format!("OnCalendar={prefix}*-*-* {h:02}:{m:02}:00")).collect())
}

#[cfg(unix)]
pub fn install(times: &[String], days: &[bool; 7]) -> Result<()> {
    let calendar = calendar_lines(times, days)?.join("\n");
    let exe = std::env::current_exe()?;
    let dir = directories::BaseDirs::new().context("Kein Home")?.config_dir().join("systemd/user");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("restic-backup-app.service"),
        format!("[Unit]\nDescription=restic Backup auf NAS\n\n[Service]\nType=oneshot\nExecStart=\"{}\" --run-backup\n", exe.display()),
    )?;
    std::fs::write(
        dir.join("restic-backup-app.timer"),
        format!("[Unit]\nDescription=Zeitplan für restic Backup\n\n[Timer]\n{calendar}\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"),
    )?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", "restic-backup-app.timer"])
}

#[cfg(unix)]
pub fn remove() -> Result<()> {
    run("systemctl", &["--user", "disable", "--now", "restic-backup-app.timer"])
}

/// Windows: je Uhrzeit ein eigener Task (`restic-backup-app-1`, `-2`, …).
#[cfg(windows)]
pub fn install(times: &[String], days: &[bool; 7]) -> Result<()> {
    if !days.iter().any(|d| *d) {
        bail!("Kein Wochentag gewählt");
    }
    let parsed = parsed_times(times)?;
    let exe = std::env::current_exe()?;
    remove().ok();
    let cmd = format!("\"{}\" --run-backup", exe.display());
    let all = days.iter().all(|d| *d);
    let day_list = DAYS_WINDOWS.iter().zip(days).filter(|(_, on)| **on).map(|(n, _)| *n).collect::<Vec<_>>().join(",");
    for (i, (h, m)) in parsed.iter().enumerate() {
        let (name, st) = (format!("restic-backup-app-{}", i + 1), format!("{h:02}:{m:02}"));
        let mut args = vec!["/create", "/f", "/sc", if all { "daily" } else { "weekly" }, "/st", &st, "/tn", &name, "/tr", &cmd];
        if !all {
            args.extend(["/d", &day_list]);
        }
        run("schtasks", &args)?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn remove() -> Result<()> {
    // Alte (v0.2) und aktuelle Tasks entfernen; fehlende Tasks sind kein Fehler
    let _ = run("schtasks", &["/delete", "/f", "/tn", "restic-backup-app"]);
    for i in 1..=24 {
        let _ = run("schtasks", &["/delete", "/f", "/tn", &format!("restic-backup-app-{i}")]);
    }
    Ok(())
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("{cmd} nicht startbar"))?;
    if !out.status.success() {
        bail!("{cmd} fehlgeschlagen: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn time() {
        assert_eq!(parse_time("02:30").unwrap(), (2, 30));
        assert!(parse_time("25:00").is_err());
        assert!(parse_time("abc").is_err());
    }

    #[test]
    fn calendar_every_day_sorted_unique() {
        let t = vec!["14:00".to_string(), "02:00".into(), "14:00".into()];
        assert_eq!(calendar_lines(&t, &[true; 7]).unwrap(), vec!["OnCalendar=*-*-* 02:00:00", "OnCalendar=*-*-* 14:00:00"]);
    }

    #[test]
    fn calendar_weekdays_only() {
        let days = [true, true, true, true, true, false, false];
        let l = calendar_lines(&["08:30".to_string()], &days).unwrap();
        assert_eq!(l, vec!["OnCalendar=Mon,Tue,Wed,Thu,Fri *-*-* 08:30:00"]);
        assert!(calendar_lines(&["08:30".to_string()], &[false; 7]).is_err());
    }
}
