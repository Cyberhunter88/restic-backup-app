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

/// Pfad, den der Zeitplan startet. Bei einem AppImage ist `current_exe()` ein temporärer
/// Mount-Pfad (`/tmp/.mount_…`), der nach dem Beenden verschwindet – dort zählt `$APPIMAGE`.
fn exe_from(appimage: Option<String>, current: std::path::PathBuf) -> std::path::PathBuf {
    match appimage {
        Some(p) if !p.is_empty() => p.into(),
        _ => current,
    }
}

pub fn exe_path() -> Result<std::path::PathBuf> {
    Ok(exe_from(std::env::var("APPIMAGE").ok(), std::env::current_exe()?))
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
    let exe = exe_path()?;
    let dir = directories::BaseDirs::new().context("Kein Home")?.config_dir().join("systemd/user");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("restic-backup-app.service"),
        format!("[Unit]\nDescription=restic Backup auf NAS\n\n[Service]\nType=oneshot\nExecStart=\"{}\" --run-backup\n", exe.display()),
    )?;
    std::fs::write(
        dir.join("restic-backup-app.timer"),
        format!("[Unit]\nDescription=Zeitplan für restic Backup (verpasste Läufe werden beim nächsten Start nachgeholt)\n\n[Timer]\n{calendar}\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"),
    )?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", "restic-backup-app.timer"])
}

#[cfg(unix)]
pub fn remove() -> Result<()> {
    run("systemctl", &["--user", "disable", "--now", "restic-backup-app.timer"])
}

/// Windows: ein Task mit einem Trigger je Uhrzeit. `StartWhenAvailable` holt verpasste Läufe
/// nach (PC war aus), das geht nur über die Task-Scheduler-Cmdlets, nicht über `schtasks`.
#[cfg(windows)]
pub fn install(times: &[String], days: &[bool; 7]) -> Result<()> {
    const DAYS_PS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    if !days.iter().any(|d| *d) {
        bail!("Kein Wochentag gewählt");
    }
    let parsed = parsed_times(times)?;
    let exe = exe_path()?.display().to_string().replace('\'', "''");
    remove().ok();
    let all = days.iter().all(|d| *d);
    let day_list = DAYS_PS.iter().zip(days).filter(|(_, on)| **on).map(|(n, _)| *n).collect::<Vec<_>>().join(",");
    let triggers = parsed
        .iter()
        .map(|(h, m)| if all { format!("New-ScheduledTaskTrigger -Daily -At '{h:02}:{m:02}'") } else { format!("New-ScheduledTaskTrigger -Weekly -DaysOfWeek {day_list} -At '{h:02}:{m:02}'") })
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "$ErrorActionPreference='Stop'; $a = New-ScheduledTaskAction -Execute '{exe}' -Argument '--run-backup'; $t = @({triggers}); \
         $s = New-ScheduledTaskSettingsSet -StartWhenAvailable -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -MultipleInstances IgnoreNew; \
         Register-ScheduledTask -TaskName 'restic-backup-app' -Action $a -Trigger $t -Settings $s -Force | Out-Null"
    );
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])
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
    let mut c = Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().with_context(|| format!("{cmd} nicht startbar"))?;
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
    fn appimage_path_wins() {
        let cur = std::path::PathBuf::from("/tmp/.mount_x/usr/bin/app");
        assert_eq!(exe_from(Some("/home/u/App.AppImage".into()), cur.clone()), std::path::PathBuf::from("/home/u/App.AppImage"));
        assert_eq!(exe_from(Some(String::new()), cur.clone()), cur);
        assert_eq!(exe_from(None, cur.clone()), cur);
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
