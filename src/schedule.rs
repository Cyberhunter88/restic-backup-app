use anyhow::{Context, Result, bail};
use chrono::{Datelike, NaiveDateTime};
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

/// Zusätzliche Einstellungen für den Zeitplan (Laptop).
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Nur am Netzteil starten
    pub only_on_ac: bool,
}

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

/// Jüngster geplanter Termin, der nicht nach `now` liegt (höchstens 7 Tage zurück).
pub fn last_slot(now: NaiveDateTime, times: &[String], days: &[bool; 7]) -> Result<Option<NaiveDateTime>> {
    let parsed = parsed_times(times)?;
    for back in 0..=7 {
        let date = now.date() - chrono::Duration::days(back);
        if !days[date.weekday().num_days_from_monday() as usize] {
            continue;
        }
        for (h, m) in parsed.iter().rev() {
            if let Some(slot) = date.and_hms_opt(*h, *m, 0).filter(|s| *s <= now) {
                return Ok(Some(slot));
            }
        }
    }
    Ok(None)
}

/// Ist ein Backup fällig? Ja, wenn seit dem letzten geplanten Termin noch keins gelang.
pub fn is_due(last_success: Option<NaiveDateTime>, slot: Option<NaiveDateTime>) -> bool {
    slot.is_some_and(|s| last_success.is_none_or(|l| l < s))
}

/// Maskiert einen Pfad für `ExecStart="…"`: systemd wertet `%` (Specifier) und `$` (Variablen) aus,
/// `\` und `"` brauchen einen Backslash.
#[cfg_attr(windows, allow(dead_code))]
fn systemd_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$")
}

/// Dienst- und Timer-Datei für systemd. Der Timer feuert zu den Uhrzeiten, beim Start der
/// Benutzersitzung und alle 30 Minuten auf der Echtzeituhr (holt so auch das Aufwachen aus
/// dem Ruhezustand nach); ob wirklich gesichert wird, entscheidet `--scheduled`.
#[cfg_attr(windows, allow(dead_code))]
fn systemd_units(exe: &std::path::Path, calendar: &str, opts: Options) -> (String, String) {
    let ac = if opts.only_on_ac { "ConditionACPower=true\n" } else { "" };
    // Niedrige CPU- und Platten-Priorität: Das Backup soll den Rechner nicht ausbremsen
    let service = format!(
        "[Unit]\nDescription=restic Backup auf NAS\n{ac}\n[Service]\nType=oneshot\nNice=10\nIOSchedulingClass=idle\nExecStart=\"{}\" --scheduled\n",
        systemd_escape(&exe.to_string_lossy())
    );
    let timer = format!(
        "[Unit]\nDescription=Zeitplan für restic Backup (verpasste Läufe werden nachgeholt)\n\n[Timer]\n{calendar}\nOnCalendar=*:0/30\nOnStartupSec=2min\nPersistent=true\nAccuracySec=1min\nRandomizedDelaySec=30\n\n[Install]\nWantedBy=timers.target\n"
    );
    (service, timer)
}

#[cfg(unix)]
pub fn install(times: &[String], days: &[bool; 7], opts: Options) -> Result<()> {
    let calendar = calendar_lines(times, days)?.join("\n");
    let exe = exe_path()?;
    let dir = directories::BaseDirs::new().context("Kein Home")?.config_dir().join("systemd/user");
    std::fs::create_dir_all(&dir)?;
    let (service, timer) = systemd_units(&exe, &calendar, opts);
    let stamp = format!("{service}\n{timer}");
    std::fs::write(dir.join("restic-backup-app.service"), service)?;
    std::fs::write(dir.join("restic-backup-app.timer"), timer)?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", "restic-backup-app.timer"])?;
    write_stamp(&stamp)
}

#[cfg(unix)]
fn is_installed() -> bool {
    directories::BaseDirs::new().is_some_and(|d| d.config_dir().join("systemd/user/restic-backup-app.timer").exists())
}

#[cfg(unix)]
fn current_stamp(times: &[String], days: &[bool; 7], opts: Options) -> Result<String> {
    let (service, timer) = systemd_units(&exe_path()?, &calendar_lines(times, days)?.join("\n"), opts);
    Ok(format!("{service}\n{timer}"))
}

#[cfg(unix)]
pub fn remove() -> Result<()> {
    run("systemctl", &["--user", "disable", "--now", "restic-backup-app.timer"])
}

/// PowerShell-Skript, das den Windows-Task registriert: ein Trigger je Uhrzeit, dazu „bei
/// Anmeldung“, „Sitzung entsperrt“ und eine Wiederholung alle 30 Minuten. `StartWhenAvailable`
/// holt verpasste Läufe nach (PC war aus), das geht nur über die Task-Scheduler-Cmdlets, nicht
/// über `schtasks`. Ob wirklich gesichert wird, entscheidet `--scheduled`.
#[cfg_attr(unix, allow(dead_code))]
fn windows_script(exe: &str, times: &[(u32, u32)], days: &[bool; 7], opts: Options) -> String {
    const DAYS_PS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let exe = exe.replace('\'', "''");
    let all = days.iter().all(|d| *d);
    let day_list = DAYS_PS.iter().zip(days).filter(|(_, on)| **on).map(|(n, _)| *n).collect::<Vec<_>>().join(",");
    let mut triggers: Vec<String> = times
        .iter()
        .map(|(h, m)| if all { format!("(New-ScheduledTaskTrigger -Daily -At '{h:02}:{m:02}')") } else { format!("(New-ScheduledTaskTrigger -Weekly -DaysOfWeek {day_list} -At '{h:02}:{m:02}')") })
        .collect();
    triggers.push("(New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME)".into());
    triggers.push("(New-ScheduledTaskTrigger -Once -At '00:00' -RepetitionInterval (New-TimeSpan -Minutes 30) -RepetitionDuration (New-TimeSpan -Days 3650))".into());
    triggers.push("$unlock".into());
    // Standardmäßig startet ein Task nicht im Akkubetrieb; der Schalter erlaubt es
    let battery = if opts.only_on_ac { "" } else { " -AllowStartIfOnBatteries" };
    format!(
        "$ErrorActionPreference='Stop'; $a = New-ScheduledTaskAction -Execute '{exe}' -Argument '--scheduled'; \
         $c = Get-CimClass -ClassName MSFT_TaskSessionStateChangeTrigger -Namespace Root/Microsoft/Windows/TaskScheduler; \
         $unlock = New-CimInstance -CimClass $c -ClientOnly -Property @{{StateChange=8; UserId=$env:USERNAME; Enabled=$true}}; \
         $t = @({}); \
         $s = New-ScheduledTaskSettingsSet -StartWhenAvailable -RunOnlyIfNetworkAvailable -DontStopIfGoingOnBatteries{battery} -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 12); \
         Register-ScheduledTask -TaskName 'restic-backup-app' -Action $a -Trigger $t -Settings $s -Force | Out-Null",
        triggers.join(",")
    )
}

#[cfg(windows)]
pub fn install(times: &[String], days: &[bool; 7], opts: Options) -> Result<()> {
    if !days.iter().any(|d| *d) {
        bail!("Kein Wochentag gewählt");
    }
    let parsed = parsed_times(times)?;
    let script = windows_script(&exe_path()?.display().to_string(), &parsed, days, opts);
    remove().ok();
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])?;
    write_stamp(&script)
}

#[cfg(windows)]
fn is_installed() -> bool {
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", "if (Get-ScheduledTask -TaskName 'restic-backup-app' -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }"]).is_ok()
}

#[cfg(windows)]
fn current_stamp(times: &[String], days: &[bool; 7], opts: Options) -> Result<String> {
    Ok(windows_script(&exe_path()?.display().to_string(), &parsed_times(times)?, days, opts))
}

#[cfg(windows)]
pub fn remove() -> Result<()> {
    // Aktuellen und alte (v0.2: „restic-backup-app-1…24“) Tasks entfernen; fehlende sind kein Fehler
    run(
        "powershell",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "foreach ($n in @('restic-backup-app') + (1..24 | ForEach-Object { \"restic-backup-app-$_\" })) { Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue }; exit 0",
        ],
    )
}

fn stamp_path() -> Result<std::path::PathBuf> {
    Ok(crate::config::data_dir()?.join("schedule-stamp.txt"))
}

fn write_stamp(stamp: &str) -> Result<()> {
    let p = stamp_path()?;
    std::fs::create_dir_all(p.parent().context("Kein Datenverzeichnis")?)?;
    std::fs::write(p, stamp)?;
    Ok(())
}

/// Ist der Zeitplan eingerichtet, aber mit einer anderen Programmversion bzw. einem anderen Pfad
/// angelegt als dem aktuellen? Dann neu einrichten, damit nach einer Aktualisierung (oder einem
/// Verschieben der Datei) nicht der alte Task weiterläuft. Liefert `true`, wenn erneuert wurde.
pub fn refresh_if_outdated(times: &[String], days: &[bool; 7], opts: Options) -> Result<bool> {
    if !is_installed() {
        return Ok(false);
    }
    let want = current_stamp(times, days, opts)?;
    if std::fs::read_to_string(stamp_path()?).is_ok_and(|have| have == want) {
        return Ok(false);
    }
    install(times, days, opts)?;
    Ok(true)
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
    use chrono::{NaiveDate, NaiveTime};

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

    /// November 2026: der 2. ist ein Montag.
    fn at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDateTime::new(NaiveDate::from_ymd_opt(2026, 11, day).unwrap(), NaiveTime::from_hms_opt(h, m, 0).unwrap())
    }

    #[test]
    fn last_slot_picks_latest_not_after_now() {
        let t = vec!["02:00".to_string(), "14:00".into()];
        assert_eq!(last_slot(at(4, 15, 0), &t, &[true; 7]).unwrap(), Some(at(4, 14, 0)));
        assert_eq!(last_slot(at(4, 13, 59), &t, &[true; 7]).unwrap(), Some(at(4, 2, 0)));
        // vor dem ersten Termin des Tages: gestern 14:00
        assert_eq!(last_slot(at(4, 1, 0), &t, &[true; 7]).unwrap(), Some(at(3, 14, 0)));
        // genau auf dem Termin zählt er schon
        assert_eq!(last_slot(at(4, 14, 0), &t, &[true; 7]).unwrap(), Some(at(4, 14, 0)));
    }

    #[test]
    fn last_slot_skips_disabled_days() {
        // Nur Mo–Fr: am Sonntag (8.) ist der letzte Termin Freitag (6.)
        let days = [true, true, true, true, true, false, false];
        assert_eq!(last_slot(at(8, 12, 0), &["08:30".to_string()], &days).unwrap(), Some(at(6, 8, 30)));
        // Nur Montag, Montagmorgen vor dem Termin: der Montag davor (26.10.)
        let monday = [true, false, false, false, false, false, false];
        let prev = NaiveDateTime::new(NaiveDate::from_ymd_opt(2026, 10, 26).unwrap(), NaiveTime::from_hms_opt(8, 0, 0).unwrap());
        assert_eq!(last_slot(at(2, 7, 0), &["08:00".to_string()], &monday).unwrap(), Some(prev));
        assert_eq!(last_slot(at(2, 7, 0), &["08:00".to_string()], &[false; 7]).unwrap(), None);
    }

    #[test]
    fn due_only_when_no_success_since_last_slot() {
        let slot = Some(at(4, 14, 0));
        assert!(is_due(None, slot));
        assert!(is_due(Some(at(4, 13, 59)), slot));
        assert!(!is_due(Some(at(4, 14, 0)), slot));
        assert!(!is_due(Some(at(4, 20, 0)), slot));
        assert!(!is_due(None, None));
    }

    #[test]
    fn systemd_units_have_catch_up_triggers() {
        let exe = std::path::Path::new("/opt/app");
        let (svc, timer) = systemd_units(exe, "OnCalendar=*-*-* 02:00:00", Options::default());
        assert!(svc.contains("ExecStart=\"/opt/app\" --scheduled"));
        assert!(!svc.contains("ConditionACPower"));
        assert!(svc.contains("Nice=10") && svc.contains("IOSchedulingClass=idle"));
        assert_eq!(systemd_escape("/opt/50%/a$b"), "/opt/50%%/a$$b");
        for want in ["OnCalendar=*-*-* 02:00:00", "OnCalendar=*:0/30", "OnStartupSec=2min", "Persistent=true"] {
            assert!(timer.contains(want), "{want}");
        }
        let (svc, _) = systemd_units(exe, "x", Options { only_on_ac: true });
        assert!(svc.contains("ConditionACPower=true"));
    }

    #[test]
    fn windows_script_triggers_and_settings() {
        let s = windows_script("C:\\it's\\app.exe", &[(2, 0), (14, 30)], &[true; 7], Options::default());
        assert!(s.contains("-Execute 'C:\\it''s\\app.exe' -Argument '--scheduled'"));
        assert!(s.contains("-Daily -At '02:00'") && s.contains("-Daily -At '14:30'"));
        assert!(s.contains("-AtLogOn") && s.contains("-RepetitionInterval") && s.contains("$unlock"));
        assert!(s.contains("-StartWhenAvailable -RunOnlyIfNetworkAvailable") && s.contains("-AllowStartIfOnBatteries"));
        let s = windows_script("a.exe", &[(8, 0)], &[true, true, true, true, true, false, false], Options { only_on_ac: true });
        assert!(s.contains("-Weekly -DaysOfWeek Monday,Tuesday,Wednesday,Thursday,Friday -At '08:00'"));
        assert!(!s.contains("-AllowStartIfOnBatteries"));
    }
}
