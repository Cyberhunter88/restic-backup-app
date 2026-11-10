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
///
/// Wurde die Programmdatei im laufenden Betrieb ersetzt (Paket-Update), hängt Linux „ (deleted)“ an den Pfad.
fn exe_from(appimage: Option<String>, current: std::path::PathBuf) -> std::path::PathBuf {
    match appimage {
        Some(p) if !p.is_empty() => p.into(),
        _ => match current.to_str().and_then(|s| s.strip_suffix(" (deleted)")) {
            Some(s) => s.into(),
            None => current,
        },
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
    /// Rechner für einen Termin aus dem Standby wecken (nur Windows; ein systemd-Timer des Benutzers darf das nicht)
    pub wake_to_run: bool,
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

/// Nächster geplanter Termin nach `now` (höchstens 7 Tage voraus).
pub fn next_slot(now: NaiveDateTime, times: &[String], days: &[bool; 7]) -> Result<Option<NaiveDateTime>> {
    let parsed = parsed_times(times)?;
    for ahead in 0..=7 {
        let date = now.date() + chrono::Duration::days(ahead);
        if !days[date.weekday().num_days_from_monday() as usize] {
            continue;
        }
        for (h, m) in &parsed {
            if let Some(slot) = date.and_hms_opt(*h, *m, 0).filter(|s| *s > now) {
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
/// Benutzersitzung und alle 10 Minuten auf der Echtzeituhr (holt so auch das Aufwachen aus
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
        "[Unit]\nDescription=Zeitplan für restic Backup (verpasste Läufe werden nachgeholt)\n\n[Timer]\n{calendar}\nOnCalendar=*:0/10\nOnStartupSec=2min\nPersistent=true\nAccuracySec=1min\nRandomizedDelaySec=30\n\n[Install]\nWantedBy=timers.target\n"
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
    let stamp = make_stamp(&exe, &format!("{service}\n{timer}"));
    std::fs::write(dir.join("restic-backup-app.service"), service)?;
    std::fs::write(dir.join("restic-backup-app.timer"), timer)?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "restic-backup-app.timer"])?;
    // Ein schon laufender Timer rechnet erst nach einem Neustart sicher mit den neuen Uhrzeiten
    run("systemctl", &["--user", "restart", "restic-backup-app.timer"])?;
    write_stamp(&stamp)
}

#[cfg(unix)]
fn is_installed() -> bool {
    directories::BaseDirs::new().is_some_and(|d| d.config_dir().join("systemd/user/restic-backup-app.timer").exists())
}

#[cfg(unix)]
fn current_stamp(times: &[String], days: &[bool; 7], opts: Options) -> Result<String> {
    let exe = exe_path()?;
    let (service, timer) = systemd_units(&exe, &calendar_lines(times, days)?.join("\n"), opts);
    Ok(make_stamp(&exe, &format!("{service}\n{timer}")))
}

/// Entfernt Timer und Dienst samt ihrer Dateien. Blieben die Dateien liegen, hielte `is_installed` den Zeitplan
/// weiter für eingerichtet, und `refresh_if_outdated` würde ihn nach einer Aktualisierung wieder aktivieren.
#[cfg(unix)]
pub fn remove() -> Result<()> {
    let disabled = if is_installed() { run("systemctl", &["--user", "disable", "--now", "restic-backup-app.timer"]) } else { Ok(()) };
    if let Some(d) = directories::BaseDirs::new() {
        let dir = d.config_dir().join("systemd/user");
        let _ = std::fs::remove_file(dir.join("restic-backup-app.timer"));
        let _ = std::fs::remove_file(dir.join("restic-backup-app.service"));
    }
    let _ = run("systemctl", &["--user", "daemon-reload"]);
    remove_stamp();
    disabled
}

#[cfg(unix)]
pub fn status() -> Status {
    let installed = is_installed();
    let show = |unit: &str, prop: &str| run_output("systemctl", &["--user", "show", unit, "-p", prop, "--value"]).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty() && s != "n/a");
    let exe = stamp_path().ok().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| stamp_exe(&s));
    Status {
        installed,
        active: installed && show("restic-backup-app.timer", "ActiveState").as_deref() == Some("active"),
        exe_exists: exe.as_ref().is_some_and(|p| p.exists()),
        exe: exe.map(|p| p.display().to_string()),
        last_run: show("restic-backup-app.timer", "LastTriggerUSec"),
        result: show("restic-backup-app.service", "ExecMainStatus").map(|r| result_text(&r)),
    }
}

/// Name des Tasks in der Windows-Aufgabenplanung.
#[cfg_attr(unix, allow(dead_code))]
const TASK_NAME: &str = "restic-backup-app";

/// Ereignis-Abfrage für einen Trigger, schon für eine PowerShell-Zeichenkette in einfachen Anführungszeichen
/// maskiert (die Abfrage selbst kommt ohne doppelte Anführungszeichen aus).
#[cfg_attr(unix, allow(dead_code))]
fn event_query(log: &str, provider: &str, id: u32) -> String {
    format!("<QueryList><Query Id='0' Path='{log}'><Select Path='{log}'>*[System[Provider[@Name='{provider}'] and EventID={id}]]</Select></Query></QueryList>").replace('\'', "''")
}

/// PowerShell-Skript, das den Windows-Task registriert: ein Trigger je Uhrzeit, dazu Anmeldung (sofort und
/// 2 Minuten später), „Sitzung entsperrt“, Aufwachen aus dem Standby, „Netzwerk verbunden“ und eine
/// Wiederholung alle 10 Minuten. `StartWhenAvailable` holt verpasste Läufe nach (PC war aus), das geht nur
/// über die Task-Scheduler-Cmdlets, nicht über `schtasks`. Ob wirklich gesichert wird, entscheidet
/// `--scheduled`; dort wird auch das NAS geprüft, deshalb gibt es keine Netzwerk-Bedingung am Task (mit ihr
/// verfielen Trigger, solange das WLAN nach Anmeldung oder Aufwachen noch nicht stand).
#[cfg_attr(unix, allow(dead_code))]
fn windows_script(task: &str, exe: &str, times: &[(u32, u32)], days: &[bool; 7], opts: Options) -> String {
    const DAYS_PS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    let exe = exe.replace('\'', "''");
    let all = days.iter().all(|d| *d);
    let day_list = DAYS_PS.iter().zip(days).filter(|(_, on)| **on).map(|(n, _)| *n).collect::<Vec<_>>().join(",");
    let mut triggers: Vec<String> = times
        .iter()
        .map(|(h, m)| if all { format!("(New-ScheduledTaskTrigger -Daily -At '{h:02}:{m:02}')") } else { format!("(New-ScheduledTaskTrigger -Weekly -DaysOfWeek {day_list} -At '{h:02}:{m:02}')") })
        .collect();
    triggers.push("(New-ScheduledTaskTrigger -AtLogOn -User $u)".into());
    triggers.push("$late".into());
    triggers.push("(New-ScheduledTaskTrigger -Once -At '00:00' -RepetitionInterval (New-TimeSpan -Minutes 10) -RepetitionDuration (New-TimeSpan -Days 3650))".into());
    triggers.extend(["$unlock", "$wake", "$net"].map(String::from));
    // Standardmäßig startet ein Task nicht im Akkubetrieb; der Schalter erlaubt es
    let battery = if opts.only_on_ac { "" } else { " -AllowStartIfOnBatteries" };
    let wake = if opts.wake_to_run { " -WakeToRun" } else { "" };
    let wake_query = event_query("System", "Microsoft-Windows-Power-Troubleshooter", 1);
    let net_query = event_query("Microsoft-Windows-NetworkProfile/Operational", "Microsoft-Windows-NetworkProfile", 10000);
    format!(
        "$ErrorActionPreference='Stop'; $u = [Security.Principal.WindowsIdentity]::GetCurrent().Name; \
         $ns = 'Root/Microsoft/Windows/TaskScheduler'; \
         $a = New-ScheduledTaskAction -Execute '{exe}' -Argument '--scheduled'; \
         $unlock = New-CimInstance -CimClass (Get-CimClass -ClassName MSFT_TaskSessionStateChangeTrigger -Namespace $ns) -ClientOnly -Property @{{StateChange=8; UserId=$u; Enabled=$true}}; \
         $ec = Get-CimClass -ClassName MSFT_TaskEventTrigger -Namespace $ns; \
         $wake = New-CimInstance -CimClass $ec -ClientOnly -Property @{{Enabled=$true; Subscription='{wake_query}'}}; \
         $net = New-CimInstance -CimClass $ec -ClientOnly -Property @{{Enabled=$true; Subscription='{net_query}'}}; \
         $late = New-ScheduledTaskTrigger -AtLogOn -User $u; $late.Delay = 'PT2M'; \
         $t = @({}); \
         $s = New-ScheduledTaskSettingsSet -StartWhenAvailable -DontStopIfGoingOnBatteries{battery}{wake} -MultipleInstances IgnoreNew -ExecutionTimeLimit (New-TimeSpan -Hours 72); \
         Register-ScheduledTask -TaskName '{task}' -Action $a -Trigger $t -Settings $s -Force | Out-Null",
        triggers.join(",")
    )
}

#[cfg(windows)]
pub fn install(times: &[String], days: &[bool; 7], opts: Options) -> Result<()> {
    if !days.iter().any(|d| *d) {
        bail!("Kein Wochentag gewählt");
    }
    let parsed = parsed_times(times)?;
    let exe = exe_path()?;
    let script = windows_script(TASK_NAME, &exe.display().to_string(), &parsed, days, opts);
    // `-Force` ersetzt den bestehenden Task. Vorher löschen wäre riskant: Scheitert das Anlegen, gäbe es keinen mehr.
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])?;
    remove_tasks(false).ok();
    write_stamp(&make_stamp(&exe, &script))
}

#[cfg(windows)]
fn is_installed() -> bool {
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", "if (Get-ScheduledTask -TaskName 'restic-backup-app' -ErrorAction SilentlyContinue) { exit 0 } else { exit 1 }"]).is_ok()
}

#[cfg(windows)]
fn current_stamp(times: &[String], days: &[bool; 7], opts: Options) -> Result<String> {
    let exe = exe_path()?;
    Ok(make_stamp(&exe, &windows_script(TASK_NAME, &exe.display().to_string(), &parsed_times(times)?, days, opts)))
}

/// Entfernt alte Tasks (v0.2: „restic-backup-app-1…24“) und mit `current` auch den aktuellen; fehlende sind kein Fehler.
#[cfg(windows)]
fn remove_tasks(current: bool) -> Result<()> {
    let first = if current { "@('restic-backup-app') + " } else { "" };
    let script = format!("foreach ($n in {first}(1..24 | ForEach-Object {{ 'restic-backup-app-' + $_ }})) {{ Unregister-ScheduledTask -TaskName $n -Confirm:$false -ErrorAction SilentlyContinue }}; exit 0");
    run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script])
}

#[cfg(windows)]
pub fn remove() -> Result<()> {
    remove_tasks(true)?;
    remove_stamp();
    Ok(())
}

#[cfg(windows)]
pub fn status() -> Status {
    let script = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; $t = Get-ScheduledTask -TaskName 'restic-backup-app' -ErrorAction SilentlyContinue; if (-not $t) { exit 0 }; \
                  $i = $t | Get-ScheduledTaskInfo; 'state=' + $t.State; 'exe=' + $t.Actions[0].Execute; \
                  if ($i.LastRunTime -and $i.LastRunTime.Year -gt 2000) { 'last=' + $i.LastRunTime.ToString('dd.MM.yyyy HH:mm') }; 'result=' + $i.LastTaskResult";
    let mut st = parse_task_status(&run_output("powershell", &["-NoProfile", "-NonInteractive", "-Command", script]).unwrap_or_default());
    st.exe_exists = st.exe.as_ref().is_some_and(|p| std::path::Path::new(p).exists());
    st
}

/// Zustand des eingerichteten Zeitplans für die Diagnose-Anzeige.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Status {
    /// Task bzw. Timer ist vorhanden
    pub installed: bool,
    /// … und nicht deaktiviert
    pub active: bool,
    /// Programmdatei, die der Zeitplan startet
    pub exe: Option<String>,
    pub exe_exists: bool,
    /// Letzter Start durch das System (Anzeigetext)
    pub last_run: Option<String>,
    /// Ergebnis des letzten Starts (Anzeigetext)
    pub result: Option<String>,
}

/// Rückgabecode des letzten Starts als lesbarer Text (Windows meldet auch Zustände als Code).
fn result_text(code: &str) -> String {
    match code.trim() {
        "0" => "ok".into(),
        "267009" => "läuft gerade".into(),
        "267011" => "noch nie gestartet".into(),
        c => match c.parse::<i64>() {
            Ok(n) => format!("Fehlercode 0x{:X}", n as u32),
            Err(_) => c.to_string(),
        },
    }
}

/// Liest die `schlüssel=wert`-Zeilen des Status-Skripts; leere Ausgabe heißt „kein Task“.
#[cfg_attr(unix, allow(dead_code))]
fn parse_task_status(out: &str) -> Status {
    let mut st = Status::default();
    for (k, v) in out.lines().filter_map(|l| l.trim().split_once('=')) {
        st.installed = true;
        match k {
            "state" => st.active = v != "Disabled",
            "exe" => st.exe = Some(v.trim_matches('"').to_string()),
            "last" => st.last_run = Some(v.to_string()),
            "result" => st.result = Some(result_text(v)),
            _ => {}
        }
    }
    st
}

fn stamp_path() -> Result<std::path::PathBuf> {
    Ok(crate::config::data_dir()?.join("schedule-stamp.txt"))
}

/// Inhalt der Stamp-Datei: die Programmdatei in der ersten Zeile, danach Skript bzw. Unit-Texte.
fn make_stamp(exe: &std::path::Path, body: &str) -> String {
    format!("exe={}\n{body}", exe.display())
}

/// Programmdatei, mit der der Zeitplan eingerichtet wurde (Stamps älterer Versionen kennen sie nicht).
fn stamp_exe(stamp: &str) -> Option<std::path::PathBuf> {
    stamp.lines().next()?.strip_prefix("exe=").map(Into::into)
}

fn remove_stamp() {
    if let Ok(p) = stamp_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Wann der Zeitplan zuletzt eingerichtet wurde (`None`: nicht eingerichtet).
pub fn activated_at() -> Option<NaiveDateTime> {
    let t = std::fs::metadata(stamp_path().ok()?).ok()?.modified().ok()?;
    Some(chrono::DateTime::<chrono::Local>::from(t).naive_local())
}

/// Liegt die Programmdatei in einem Cargo-Ausgabeordner (`…/debug/` oder `…/release/`, auch bei verlegtem
/// `CARGO_TARGET_DIR`)? Installierte Versionen liegen nie in einem Ordner dieses Namens.
fn is_dev_build(exe: &std::path::Path) -> bool {
    exe.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "debug" || n == "release")
}

/// Soll der Zeitplan seine bisherige Programmdatei behalten? Ja, wenn gerade ein Entwicklungs-Build läuft und die
/// eingetragene (andere) Datei noch existiert: Ein Probelauf aus dem Build-Ordner soll den Zeitplan nicht dorthin umbiegen.
fn keep_installed_exe(current: &std::path::Path, installed: Option<&std::path::Path>, installed_exists: bool) -> bool {
    is_dev_build(current) && installed_exists && installed.is_some_and(|i| i != current)
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
    let have = std::fs::read_to_string(stamp_path()?).unwrap_or_default();
    if have == want {
        return Ok(false);
    }
    let installed = stamp_exe(&have);
    if keep_installed_exe(&exe_path()?, installed.as_deref(), installed.as_ref().is_some_and(|p| p.exists())) {
        return Ok(false);
    }
    install(times, days, opts)?;
    Ok(true)
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    run_output(cmd, args).map(|_| ())
}

/// Führt den Befehl aus und liefert seine Standardausgabe.
fn run_output(cmd: &str, args: &[&str]) -> Result<String> {
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
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
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
        assert_eq!(exe_from(None, "/usr/bin/app (deleted)".into()), std::path::PathBuf::from("/usr/bin/app"));
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
        for want in ["OnCalendar=*-*-* 02:00:00", "OnCalendar=*:0/10", "OnStartupSec=2min", "Persistent=true"] {
            assert!(timer.contains(want), "{want}");
        }
        let (svc, _) = systemd_units(exe, "x", Options { only_on_ac: true, ..Default::default() });
        assert!(svc.contains("ConditionACPower=true"));
    }

    #[test]
    fn windows_script_triggers_and_settings() {
        let s = windows_script("restic-backup-app", "C:\\it's\\app.exe", &[(2, 0), (14, 30)], &[true; 7], Options::default());
        assert!(s.contains("-Execute 'C:\\it''s\\app.exe' -Argument '--scheduled'"));
        assert!(s.contains("-Daily -At '02:00'") && s.contains("-Daily -At '14:30'"));
        assert!(s.contains("-AtLogOn -User $u),$late,") && s.contains("$late.Delay = 'PT2M'"));
        assert!(s.contains("-RepetitionInterval (New-TimeSpan -Minutes 10)"));
        assert!(s.ends_with("-TaskName 'restic-backup-app' -Action $a -Trigger $t -Settings $s -Force | Out-Null"));
        // Entsperren, Aufwachen und „Netzwerk verbunden“
        assert!(s.contains(",$unlock,$wake,$net)"));
        assert!(s.contains("Subscription='<QueryList><Query Id=''0'' Path=''System''><Select Path=''System''>*[System[Provider[@Name=''Microsoft-Windows-Power-Troubleshooter''] and EventID=1]]</Select></Query></QueryList>'"));
        assert!(s.contains("Path=''Microsoft-Windows-NetworkProfile/Operational''") && s.contains("EventID=10000"));
        // Keine doppelten Anführungszeichen (die Befehlszeile bräuchte sonst eine weitere Maskierung)
        assert!(!s.contains('"'));
        assert!(s.contains("-StartWhenAvailable -DontStopIfGoingOnBatteries -AllowStartIfOnBatteries -MultipleInstances"));
        assert!(!s.contains("RunOnlyIfNetworkAvailable") && !s.contains("-WakeToRun"));
        let s = windows_script("x", "a.exe", &[(8, 0)], &[true, true, true, true, true, false, false], Options { only_on_ac: true, wake_to_run: true });
        assert!(s.contains("-Weekly -DaysOfWeek Monday,Tuesday,Wednesday,Thursday,Friday -At '08:00'"));
        assert!(!s.contains("-AllowStartIfOnBatteries") && s.contains("-DontStopIfGoingOnBatteries -WakeToRun"));
    }

    #[test]
    fn next_slot_picks_first_after_now() {
        let t = vec!["02:00".to_string(), "14:00".into()];
        assert_eq!(next_slot(at(4, 13, 0), &t, &[true; 7]).unwrap(), Some(at(4, 14, 0)));
        // genau auf dem Termin: der nächste
        assert_eq!(next_slot(at(4, 14, 0), &t, &[true; 7]).unwrap(), Some(at(5, 2, 0)));
        // Nur Mo–Fr: am Freitagabend (6.) ist der nächste Termin Montag (9.)
        let days = [true, true, true, true, true, false, false];
        assert_eq!(next_slot(at(6, 20, 0), &["08:30".to_string()], &days).unwrap(), Some(at(9, 8, 30)));
        assert_eq!(next_slot(at(6, 20, 0), &t, &[false; 7]).unwrap(), None);
    }

    #[test]
    fn stamp_keeps_exe_and_dev_build_does_not_take_over() {
        use std::path::Path;
        let stamp = make_stamp(Path::new("/opt/app"), "Skript");
        assert_eq!(stamp_exe(&stamp), Some("/opt/app".into()));
        // Stamp einer älteren Version: Programmdatei unbekannt
        assert_eq!(stamp_exe("$ErrorActionPreference='Stop'"), None);
        let (dev, inst) = (Path::new("/home/u/proj/target/debug/app"), Path::new("/opt/app"));
        assert!(keep_installed_exe(dev, Some(inst), true));
        assert!(keep_installed_exe(Path::new("/tmp/build/release/app"), Some(inst), true));
        // eingetragene Datei fehlt, unbekannt oder ist dieselbe: neu einrichten
        assert!(!keep_installed_exe(dev, Some(inst), false));
        assert!(!keep_installed_exe(dev, None, false));
        assert!(!keep_installed_exe(dev, Some(dev), true));
        // installierte Version ersetzt immer
        assert!(!keep_installed_exe(inst, Some(dev), true));
    }

    #[test]
    fn task_status_parsed() {
        assert_eq!(parse_task_status(""), Status::default());
        let st = parse_task_status("state=Ready\r\nexe=\"C:\\Program Files\\app.exe\"\r\nlast=10.11.2026 20:00\r\nresult=0\r\n");
        assert!(st.installed && st.active);
        assert_eq!(st.exe.as_deref(), Some("C:\\Program Files\\app.exe"));
        assert_eq!(st.last_run.as_deref(), Some("10.11.2026 20:00"));
        assert_eq!(st.result.as_deref(), Some("ok"));
        let st = parse_task_status("state=Disabled\nexe=a.exe\nresult=267011\n");
        assert!(st.installed && !st.active && st.last_run.is_none());
        assert_eq!(st.result.as_deref(), Some("noch nie gestartet"));
        assert_eq!(result_text("2147942402"), "Fehlercode 0x80070002");
    }

    /// Registriert den Task unter einem Testnamen in der echten Aufgabenplanung, prüft die Trigger und löscht
    /// ihn wieder: `cargo test live_register -- --ignored`
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_register() {
        let name = "restic-backup-app-test";
        let script = windows_script(name, "C:\\Windows\\System32\\cmd.exe", &[(2, 0), (14, 30)], &[true, false, true, false, true, false, false], Options { only_on_ac: false, wake_to_run: true });
        run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &script]).unwrap();
        let read = format!("$t = Get-ScheduledTask -TaskName '{name}'; ($t.Triggers | ForEach-Object {{ $_.CimClass.CimClassName }}) -join ','; 'net=' + $t.Settings.RunOnlyIfNetworkAvailable; 'wake=' + $t.Settings.WakeToRun; 'rep=' + ($t.Triggers.Repetition.Interval -join ''); 'delay=' + ($t.Triggers.Delay -join '')");
        let out = run_output("powershell", &["-NoProfile", "-NonInteractive", "-Command", &read]);
        run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &format!("Unregister-ScheduledTask -TaskName '{name}' -Confirm:$false")]).unwrap();
        let out = out.unwrap();
        println!("{out}");
        assert!(out.contains("MSFT_TaskWeeklyTrigger,MSFT_TaskWeeklyTrigger,MSFT_TaskLogonTrigger,MSFT_TaskLogonTrigger,MSFT_TaskTimeTrigger,MSFT_TaskSessionStateChangeTrigger,MSFT_TaskEventTrigger,MSFT_TaskEventTrigger"));
        assert!(out.contains("net=False") && out.contains("wake=True") && out.contains("rep=PT10M") && out.contains("delay=PT2M"));
    }
}
