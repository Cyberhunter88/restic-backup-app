use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    pub host: String,
    pub share: String,
    /// Unterordner innerhalb der Freigabe, in dem das restic-Repo liegt
    pub subpath: String,
    pub smb_user: String,
    pub sources: Vec<String>,
    /// Jeden Quellordner als eigenen Snapshot sichern (statt aller Ordner in einem)
    pub separate_snapshots: bool,
    pub excludes: Vec<String>,
    /// Verzeichnisse mit CACHEDIR.TAG ausschließen
    pub exclude_caches: bool,
    /// Nicht in andere Dateisysteme wechseln: eingehängte Laufwerke und Netzfreigaben unter einem Quellordner
    /// werden nicht mitgesichert (nur Linux; Standard an)
    pub one_file_system: bool,
    /// Vor dem Sichern nicht erst alle Dateien zählen (`--no-scan`): schneller, aber ohne Prozentanzeige
    pub no_scan: bool,
    /// Ordner, die eine dieser Dateien enthalten, werden ausgeschlossen (z. B. `.nobackup`)
    pub exclude_if_present: Vec<String>,
    /// Dateien größer als dieser Wert werden ausgeschlossen (restic-Format, z. B. `500M`); leer = aus
    pub exclude_larger_than: String,
    /// Neues Repository anlegen, falls am Pfad keins existiert
    pub allow_init: bool,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    /// Alle so viele Tage prüft ein Zeitplan-Lauf einen Teil der Repo-Daten (0 = aus)
    pub check_interval_days: u32,
    /// Anteil der Daten (Prozent), der bei der regelmäßigen Prüfung gelesen wird
    pub check_subset_percent: u32,
    /// Uhrzeiten (HH:MM), zu denen das Backup läuft
    pub schedule_times: Vec<String>,
    /// Wochentage (Mo..So), an denen der Zeitplan gilt
    pub schedule_days: [bool; 7],
    /// Zeitplan-Läufe nur am Netzteil starten (Laptop)
    pub only_on_ac: bool,
    /// Rechner für einen geplanten Termin aus dem Standby wecken (nur Windows)
    pub wake_to_run: bool,
    /// Meldung, wenn so viele Tage kein Backup gelungen ist (0 = aus)
    pub warn_after_days: u32,
    /// Warnung nach dem Backup, wenn auf dem NAS weniger als so viele GB frei sind (0 = aus)
    pub warn_free_gb: u32,
    /// Auch nach einem erfolgreichen Lauf eine Desktop-Meldung senden (sonst nur bei Warnung/Fehler)
    pub notify_success: bool,
    /// Fenster schließen schickt die App in den Tray (nur mit Tray); sonst beendet es sie
    pub close_to_tray: bool,
    /// Beim Start der App (höchstens einmal täglich) nach einer neuen Version suchen
    pub check_updates: bool,
    /// Alte Einstellung (nur zum Übernehmen älterer Konfigurationen)
    #[serde(skip_serializing)]
    schedule_time: String,
    pub restic_bin: String,
    /// Fester Rechnername für Snapshots (leer = Name des Rechners). Verhindert, dass eine Umbenennung
    /// die Parent-Kette bricht (dann würde alles neu eingelesen) und `forget` alte Snapshots übersieht.
    pub backup_host: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: String::new(),
            share: String::new(),
            subpath: "restic-repo".into(),
            smb_user: String::new(),
            sources: vec![],
            separate_snapshots: false,
            excludes: vec![],
            exclude_caches: true,
            one_file_system: cfg!(unix),
            no_scan: true,
            exclude_if_present: vec![],
            exclude_larger_than: String::new(),
            allow_init: false,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            check_interval_days: 7,
            check_subset_percent: 5,
            schedule_times: vec!["02:00".into()],
            schedule_days: [true; 7],
            only_on_ac: false,
            wake_to_run: false,
            warn_after_days: 3,
            warn_free_gb: 20,
            notify_success: false,
            close_to_tray: true,
            check_updates: true,
            schedule_time: String::new(),
            restic_bin: default_restic_bin(),
            backup_host: String::new(),
        }
    }
}

/// Übliche Ausschlüsse für alle Systeme: Build-Artefakte, Caches, temporäre Dateien.
const COMMON_ALL: &[&str] = &[
    // Allgemein: Build-Artefakte, Caches, temporäre Dateien
    "**/node_modules", "**/target", "**/__pycache__", "**/.pytest_cache", "**/.venv", "**/.gradle",
    "**/.cache", "*.tmp", "*.temp", "*.part", "*.crdownload", "*.swp", "~$*", "*.dmp", "**/.DS_Store",
    "**/.mypy_cache", "**/.ruff_cache", "**/.tox", "**/.next", "**/.terraform", "**/.nuget/packages", "**/.m2/repository",
    "**/.rustup/toolchains", "**/.android/cache", "**/.ccache",
    // Allgemein: Browser-, Electron- und Spiele-Caches (liegen je nach System unter AppData bzw. ~/.config)
    "**/Code Cache", "**/GPUCache", "**/ShaderCache", "**/DawnGraphiteCache", "**/DawnWebGPUCache", "**/Service Worker/CacheStorage",
    "**/GrShaderCache", "**/Crashpad", "**/DXCache", "**/GLCache",
    // Allgemein: Sperrdateien laufender Programme (Browser-Datenbanken, Codex); sie sind unter Windows nicht lesbar
    // und würden das Backup jedes Mal mit einer Warnung beenden
    "**/LOCK", "**/.codex/tmp", "**/.codex/*.lock", "**/.codex/.codex-provisioning-*.guard",
];

/// Übliche Ausschlüsse nur für Windows (System-Dateien, AppData).
const COMMON_WINDOWS: &[&str] = &[
    // Windows: System- und Papierkorbdateien
    "**/$RECYCLE.BIN", "**/System Volume Information", "**/pagefile.sys", "**/swapfile.sys", "**/hiberfil.sys",
    "**/Thumbs.db", "**/desktop.ini", "**/NTUSER.DAT*", "**/UsrClass.dat*",
    // restic unterscheidet Groß- und Kleinschreibung: die Registry-Dateien heißen auf der Platte klein (ntuser.dat.LOG1)
    "**/ntuser.dat*", "**/ntuser.ini", "**/usrclass.dat*",
    // Windows: AppData (Temp, Caches, Store-Apps, Browser-Caches)
    "**/AppData/Local/Temp", "**/AppData/Local/Microsoft/Windows/INetCache", "**/AppData/Local/Microsoft/Windows/Explorer",
    "**/AppData/Local/Microsoft/WindowsApps", "**/AppData/Local/Packages", "**/AppData/Local/CrashDumps",
    "**/AppData/Local/D3DSCache", "**/AppData/Local/pip/cache", "**/AppData/Local/npm-cache",
    "**/AppData/Local/Google/Chrome/User Data/*/Cache", "**/AppData/Local/Microsoft/Edge/User Data/*/Cache",
    "**/AppData/Roaming/Code/Cache*", "**/AppData/Roaming/Code/CachedData", "**/AppData/Roaming/Code/CachedExtensionVSIXs",
    "**/AppData/Local/Microsoft/Windows/WebCache", "**/AppData/Local/Microsoft/Windows/Temporary Internet Files",
    "**/AppData/Local/Microsoft/Windows/Caches", "**/AppData/Local/Microsoft/Windows/DeliveryOptimization",
    "**/AppData/Local/Microsoft/CLR_v4.0*", "**/AppData/Local/Microsoft/Edge/User Data/*/Code Cache",
    "**/AppData/Local/BraveSoftware/*/User Data/*/Cache", "**/AppData/Local/Mozilla/Firefox/Profiles/*/cache2",
    "**/AppData/Local/NVIDIA", "**/AppData/Roaming/NVIDIA/ComputeCache", "**/AppData/Local/AMD/DxCache",
    "**/AppData/Local/pip", "**/AppData/Local/NuGet", "**/AppData/Local/Programs", "**/AppData/Local/*-updater",
    "**/AppData/Local/FortniteGame", "**/AppData/Local/EpicGamesLauncher/Saved/webcache*", "**/AppData/Local/Steam/htmlcache",
    "**/AppData/Roaming/discord/Cache", "**/AppData/Roaming/Spotify/Storage", "**/AppData/Local/Spotify/Storage",
    "**/AppData/LocalLow/Temp", "**/scoop/cache", "**/.docker/desktop",
];

/// Übliche Ausschlüsse nur für Linux (Caches, Papierkorb, Systemordner).
const COMMON_LINUX: &[&str] = &[
    // Linux: Caches, Papierkorb, Laufzeitdaten
    "**/.local/share/Trash", "**/.local/share/Steam", "**/.thumbnails", "**/.npm/_cacache", "**/.cargo/registry",
    "**/.config/*/Cache", "**/.config/*/Code Cache", "**/.config/*/GPUCache", "**/.mozilla/firefox/*/cache2",
    "**/.var/app/*/cache", "**/snap/*/common/.cache", "**/.local/share/flatpak", "**/.local/share/containers",
    "**/.local/share/baloo", "**/.steam", "**/.wine/drive_c/windows", "**/.gvfs", "**/.dbus",
    "**/.xsession-errors*", "**/.local/share/docker", "**/.config/google-chrome/*/Cache", "**/.config/BraveSoftware/*/*/Cache",
    "**/.config/chromium/*/Cache", "**/.config/discord/Cache", "**/.config/Code/CachedData", "**/.config/Code/CachedExtensionVSIXs",
    "/proc", "/sys", "/dev", "/run", "/tmp", "/var/tmp", "/var/cache", "/lost+found",
];

/// Übliche Ausschlüsse für das System, auf dem die App läuft (allgemeine plus Windows- bzw. Linux-Muster).
pub fn common_excludes() -> Vec<&'static str> {
    let system = if cfg!(windows) { COMMON_WINDOWS } else { COMMON_LINUX };
    COMMON_ALL.iter().chain(system).copied().collect()
}

/// Name des Systems für die Beschriftung des Buttons.
pub const SYSTEM_NAME: &str = if cfg!(windows) { "Windows" } else { "Linux" };

/// Zerlegt eingefügten Text (ein Muster pro Zeile) und hängt neue, nicht doppelte Muster an.
/// Leerzeilen und Kommentarzeilen (`#`) werden übersprungen. Liefert die Anzahl hinzugefügter Muster.
pub fn add_excludes(list: &mut Vec<String>, text: &str) -> usize {
    let mut added = 0;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || list.iter().any(|e| e == line) {
            continue;
        }
        list.push(line.to_string());
        added += 1;
    }
    added
}

fn restic_works(p: &std::path::Path) -> bool {
    let mut c = std::process::Command::new(p);
    c.arg("version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output().map(|o| o.status.success()).unwrap_or(false)
}

/// Bevorzugt ein neben der Anwendung liegendes restic, sonst das aus dem PATH.
pub fn default_restic_bin() -> String {
    let name = if cfg!(windows) { "restic.exe" } else { "restic" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent().map(|d| d.join(name)) {
            if p.exists() && restic_works(&p) {
                return p.to_string_lossy().into_owned();
            }
        }
    }
    "restic".into()
}

fn dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("", "", "restic-backup-app").context("Kein Home-Verzeichnis gefunden")
}

pub fn config_path() -> Result<PathBuf> {
    Ok(dirs()?.config_dir().join("config.toml"))
}

pub fn data_dir() -> Result<PathBuf> {
    Ok(dirs()?.data_local_dir().to_path_buf())
}

pub fn last_run_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("last-run.log"))
}

pub fn history_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("history.tsv"))
}

pub fn last_check_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("last-check.txt"))
}

/// Ergebnis der letzten Repository-Prüfung.
#[derive(Debug, Clone, PartialEq)]
pub struct LastCheck {
    pub time: chrono::NaiveDateTime,
    pub ok: bool,
}

const CHECK_FMT: &str = "%Y-%m-%d %H:%M:%S";

pub fn write_last_check(ok: bool) {
    if let Ok(p) = last_check_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = write_atomic(&p, &format!("{}\t{}\n", chrono::Local::now().format(CHECK_FMT), if ok { "ok" } else { "fehler" }));
    }
}

pub fn read_last_check() -> Option<LastCheck> {
    parse_last_check(&std::fs::read_to_string(last_check_path().ok()?).ok()?)
}

fn parse_last_check(text: &str) -> Option<LastCheck> {
    let (t, r) = text.trim().split_once('\t')?;
    Some(LastCheck { time: chrono::NaiveDateTime::parse_from_str(t, CHECK_FMT).ok()?, ok: r == "ok" })
}

/// Ist eine Prüfung fällig? Ohne bisherige Prüfung: ja (sofern aktiviert).
pub fn check_due(last: Option<&LastCheck>, interval_days: u32, now: chrono::NaiveDateTime) -> bool {
    interval_days > 0 && last.is_none_or(|l| now.signed_duration_since(l.time).num_days() >= interval_days as i64)
}

/// Abstand in Tagen, in dem `forget` zusätzlich mit `--prune` läuft (Prune liest über SMB den ganzen Index).
pub const PRUNE_INTERVAL_DAYS: i64 = 7;

pub fn last_prune_path() -> Result<PathBuf> {
    Ok(data_dir()?.join("last-prune.txt"))
}

pub fn write_last_prune() {
    if let Ok(p) = last_prune_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap());
        let _ = write_atomic(&p, &format!("{}	ok
", chrono::Local::now().format(CHECK_FMT)));
    }
}

pub fn read_last_prune() -> Option<LastCheck> {
    parse_last_check(&std::fs::read_to_string(last_prune_path().ok()?).ok()?)
}

/// Ist wieder ein Prune fällig? Ohne bisherigen Prune: ja.
pub fn prune_due(last: Option<&LastCheck>, now: chrono::NaiveDateTime) -> bool {
    last.is_none_or(|l| now.signed_duration_since(l.time).num_days() >= PRUNE_INTERVAL_DAYS)
}

/// Zeitstempel-Dateien im Datenverzeichnis (letzter Erfolg, Pause, Hinweise …).
const TS_FMT: &str = "%Y-%m-%d %H:%M:%S";

/// Schreibt erst in eine Nachbardatei und benennt sie dann um, damit ein Absturz oder Stromausfall
/// mitten im Schreiben keine halbe (unlesbare) Datei hinterlässt.
fn write_atomic(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, content)?;
    std::fs::rename(&tmp, path).inspect_err(|_| drop(std::fs::remove_file(&tmp)))
}

fn read_ts(name: &str) -> Option<chrono::NaiveDateTime> {
    let text = std::fs::read_to_string(data_dir().ok()?.join(name)).ok()?;
    chrono::NaiveDateTime::parse_from_str(text.trim(), TS_FMT).ok()
}

fn write_ts(name: &str, t: chrono::NaiveDateTime) {
    if let Ok(d) = data_dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = write_atomic(&d.join(name), &format!("{}\n", t.format(TS_FMT)));
    }
}

fn now() -> chrono::NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// Zeitpunkt des letzten erfolgreichen Backups (auch mit Warnungen).
pub fn read_last_success() -> Option<chrono::NaiveDateTime> {
    read_ts("last-success.txt")
}

const SCHEDULE_FAILURES_FILE: &str = "schedule-failures.txt";

/// Liegt der letzte Fehlschlag länger zurück, beginnt die Zählung neu (der Fehler von gestern soll die
/// Wartezeit heute nicht gleich auf das Maximum setzen).
const FAILURE_STREAK_HOURS: i64 = 6;

fn parse_schedule_failures(text: &str) -> Option<(u32, chrono::NaiveDateTime)> {
    let (n, t) = text.trim().split_once('\t')?;
    Some((n.parse().ok()?, chrono::NaiveDateTime::parse_from_str(t, TS_FMT).ok()?))
}

/// Zählerstand nach einem weiteren Fehlschlag zum Zeitpunkt `now`.
fn next_failure_count(prev: Option<(u32, chrono::NaiveDateTime)>, now: chrono::NaiveDateTime) -> u32 {
    match prev {
        Some((n, t)) if now.signed_duration_since(t).num_hours() < FAILURE_STREAK_HOURS => n.saturating_add(1),
        _ => 1,
    }
}

/// Fehlgeschlagene Zeitplan-Läufe in Folge und der Zeitpunkt des letzten (der Zeitplan wartet danach gestaffelt).
pub fn read_schedule_failures() -> Option<(u32, chrono::NaiveDateTime)> {
    parse_schedule_failures(&std::fs::read_to_string(data_dir().ok()?.join(SCHEDULE_FAILURES_FILE)).ok()?)
}

/// Verbucht einen fehlgeschlagenen Zeitplan-Lauf und liefert, der wievielte in Folge es ist.
pub fn record_schedule_failure() -> u32 {
    let t = now();
    let n = next_failure_count(read_schedule_failures(), t);
    if let Ok(d) = data_dir() {
        let _ = std::fs::create_dir_all(&d);
        let _ = write_atomic(&d.join(SCHEDULE_FAILURES_FILE), &format!("{n}\t{}\n", t.format(TS_FMT)));
    }
    n
}

pub fn clear_schedule_failures() {
    if let Ok(d) = data_dir() {
        let _ = std::fs::remove_file(d.join(SCHEDULE_FAILURES_FILE));
    }
}

/// So viele Zeilen behält das Zeitplan-Protokoll.
const SCHEDULE_LOG_MAX: usize = 200;

/// Hängt `msg` an. Wiederholt sich die letzte Meldung, bekommt sie nur den neuen Zeitstempel: Der Zeitplan
/// startet alle paar Minuten, „nicht fällig“ soll das Protokoll nicht füllen.
fn push_schedule_log(lines: &mut Vec<String>, ts: &str, msg: &str) {
    let msg = msg.replace(['\t', '\n', '\r'], " ");
    if lines.last().is_some_and(|l| l.split_once('\t').is_some_and(|(_, m)| m == msg)) {
        lines.pop();
    }
    lines.push(format!("{ts}\t{msg}"));
    let skip = lines.len().saturating_sub(SCHEDULE_LOG_MAX);
    lines.drain(..skip);
}

/// Schreibt eine Entscheidung des Zeitplan-Laufs nach `schedule.log` (unter Windows hat er keine Konsole).
pub fn append_schedule_log(msg: &str) {
    let Ok(d) = data_dir() else { return };
    let _ = std::fs::create_dir_all(&d);
    let p = d.join("schedule.log");
    let mut lines: Vec<String> = std::fs::read_to_string(&p).unwrap_or_default().lines().map(String::from).collect();
    push_schedule_log(&mut lines, &now().format(TS_FMT).to_string(), msg);
    let _ = write_atomic(&p, &(lines.join("\n") + "\n"));
}

/// Die letzten `n` Einträge des Zeitplan-Protokolls als `(Zeit, Meldung)`, neueste zuerst.
pub fn read_schedule_log(n: usize) -> Vec<(String, String)> {
    let Ok(d) = data_dir() else { return vec![] };
    let text = std::fs::read_to_string(d.join("schedule.log")).unwrap_or_default();
    text.lines().rev().take(n).filter_map(|l| l.split_once('\t')).map(|(t, m)| (t.to_string(), m.to_string())).collect()
}

/// Bis wann der Zeitplan pausiert ist (falls gesetzt).
pub fn read_pause() -> Option<chrono::NaiveDateTime> {
    read_ts("pause-until.txt")
}

pub fn write_pause(until: Option<chrono::NaiveDateTime>) {
    match until {
        Some(t) => write_ts("pause-until.txt", t),
        None => {
            if let Ok(d) = data_dir() {
                let _ = std::fs::remove_file(d.join("pause-until.txt"));
            }
        }
    }
}

/// Ist die Pause zum Zeitpunkt `now` noch aktiv?
pub fn pause_active(until: Option<chrono::NaiveDateTime>, now: chrono::NaiveDateTime) -> bool {
    until.is_some_and(|u| u > now)
}

/// Wann zuletzt die „Seit X Tagen kein Backup“-Meldung gesendet wurde.
pub fn read_overdue_notice() -> Option<chrono::NaiveDateTime> {
    read_ts("last-overdue-notice.txt")
}

pub fn write_overdue_notice() {
    write_ts("last-overdue-notice.txt", now());
}

/// Wann zuletzt nach einer neuen Version gesucht wurde.
pub fn read_update_check() -> Option<chrono::NaiveDateTime> {
    read_ts("update-check.txt")
}

pub fn write_update_check() {
    write_ts("update-check.txt", now());
}

/// Sperre gegen parallele Backups (GUI und Zeitplan). Eine Betriebssystem-Dateisperre,
/// die auch nach einem Absturz oder Stromausfall automatisch frei wird.
pub struct RunLock {
    _file: std::fs::File,
}

impl RunLock {
    /// `Ok(None)`: Es läuft bereits ein Backup.
    pub fn acquire() -> Result<Option<Self>> {
        let d = data_dir()?;
        std::fs::create_dir_all(&d)?;
        let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(d.join("run.lock"))?;
        match f.try_lock() {
            Ok(()) => {
                // Reste eines früheren Laufs (Absturz) dürfen weder angezeigt werden noch den neuen Lauf abbrechen
                let _ = std::fs::remove_file(d.join(RUN_STATUS_FILE));
                let _ = std::fs::remove_file(d.join(CANCEL_FLAG_FILE));
                Ok(Some(Self { _file: f }))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

impl Drop for RunLock {
    fn drop(&mut self) {
        if let Ok(d) = data_dir() {
            let _ = std::fs::remove_file(d.join(RUN_STATUS_FILE));
            let _ = std::fs::remove_file(d.join(CANCEL_FLAG_FILE));
        }
    }
}

const RUN_STATUS_FILE: &str = "run-status.txt";
const CANCEL_FLAG_FILE: &str = "cancel.flag";

/// Zustand eines Backups, das in einem anderen Prozess läuft (Zeitplan oder `--run-backup`).
/// Der Prozess schreibt ihn in `run-status.txt`, solange er die `RunLock` hält.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunStatus {
    pub started: Option<chrono::NaiveDateTime>,
    /// 0.0 bis 1.0
    pub percent: f32,
    /// Aktueller Schritt
    pub text: String,
}

fn format_run_status(s: &RunStatus) -> String {
    let started = s.started.map(|t| t.format(TS_FMT).to_string()).unwrap_or_default();
    format!("{started}\t{:.4}\t{}\n", s.percent, s.text.replace(['\t', '\n', '\r'], " "))
}

fn parse_run_status(text: &str) -> RunStatus {
    let mut parts = text.trim_end().splitn(3, '\t');
    let started = parts.next().and_then(|t| chrono::NaiveDateTime::parse_from_str(t, TS_FMT).ok());
    let percent = parts.next().and_then(|p| p.parse::<f32>().ok()).unwrap_or(0.0).clamp(0.0, 1.0);
    RunStatus { started, percent, text: parts.next().unwrap_or("").to_string() }
}

/// Meldet den Fortschritt des laufenden Backups an andere Prozesse (die Fenster-Anzeige).
/// Nur aufrufen, solange die eigene `RunLock` gehalten wird.
pub fn write_run_status(s: &RunStatus) {
    if let Ok(d) = data_dir() {
        let _ = write_atomic(&d.join(RUN_STATUS_FILE), &format_run_status(s));
    }
}

/// Läuft ein Backup in einem anderen Prozess? Nur gültig, wenn dieser Prozess selbst keins hält.
///
/// Solange keine Statusdatei da ist, wird die Sperre gar nicht angefasst: Eine Probe hielte sie kurz selbst, und
/// ein Zeitplan-Lauf, der genau dann startet, würde sich mit „läuft bereits“ beenden.
pub fn read_external_run() -> Option<RunStatus> {
    let d = data_dir().ok()?;
    let path = d.join(RUN_STATUS_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let lock = std::fs::OpenOptions::new().write(true).open(d.join("run.lock")).ok()?;
    match lock.try_lock() {
        Err(std::fs::TryLockError::WouldBlock) => Some(parse_run_status(&text)),
        Ok(()) => {
            // Niemand hält die Sperre: Die Datei blieb von einem abgestürzten Lauf übrig. Solange wir die Sperre
            // halten, kann kein neuer Lauf sie gerade geschrieben haben, sie darf also weg.
            let _ = std::fs::remove_file(&path);
            None
        }
        Err(std::fs::TryLockError::Error(_)) => None,
    }
}

/// Bittet den laufenden Backup-Prozess (Zeitplan), abzubrechen.
pub fn request_cancel() {
    if let Ok(d) = data_dir() {
        let _ = std::fs::write(d.join(CANCEL_FLAG_FILE), "");
    }
}

/// Wurde der Abbruch angefordert? Löscht die Anforderung.
pub fn take_cancel_request() -> bool {
    data_dir().is_ok_and(|d| std::fs::remove_file(d.join(CANCEL_FLAG_FILE)).is_ok())
}

/// Sperre für das Fenster: verhindert, dass die App zweimal geöffnet wird (Windows und Linux).
/// Die OS-Dateisperre fällt weg, sobald der Prozess endet (auch bei Absturz).
pub struct InstanceLock {
    _file: std::fs::File,
}

impl InstanceLock {
    /// `Ok(None)`: Die App läuft bereits.
    pub fn acquire() -> Result<Option<Self>> {
        let d = data_dir()?;
        std::fs::create_dir_all(&d)?;
        let f = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(d.join("gui.lock"))?;
        match f.try_lock() {
            Ok(()) => {
                // Eine alte Anforderung (von einem früheren Start) soll das Fenster nicht gleich zeigen
                let _ = std::fs::remove_file(d.join("show-window.flag"));
                Ok(Some(Self { _file: f }))
            }
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }
}

/// Bittet die laufende App, ihr Fenster zu zeigen (zweiter Start).
pub fn request_show_window() {
    if let Ok(d) = data_dir() {
        let _ = std::fs::write(d.join("show-window.flag"), "");
    }
}

/// Wurde das Fenster angefordert? Löscht die Anforderung.
pub fn take_show_window_request() -> bool {
    data_dir().is_ok_and(|d| std::fs::remove_file(d.join("show-window.flag")).is_ok())
}

/// Ein Eintrag im lokalen Verlauf.
pub struct HistoryEntry {
    pub time: String,
    pub result: RunResult,
    pub summary: String,
}

/// Eine Zeile aus `history.tsv` (`Zeit<TAB>ok|fehler|abbruch<TAB>Zusammenfassung`).
fn parse_history_line(l: &str) -> Option<HistoryEntry> {
    let mut it = l.splitn(3, '\t');
    let time = it.next()?.into();
    let result = match it.next()? {
        "ok" => RunResult::Ok,
        "abbruch" => RunResult::Cancelled,
        _ => RunResult::Failed,
    };
    Some(HistoryEntry { time, result, summary: it.next().unwrap_or("").into() })
}

const HISTORY_MAX: usize = 200;

/// Ausgang eines Backup-Laufs. Ein Abbruch durch den Nutzer ist kein Fehlschlag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RunResult {
    Ok,
    Failed,
    Cancelled,
}

fn append_history(result: RunResult, lines: &[String]) -> Result<()> {
    let p = history_path()?;
    let summary = lines.iter().rev().find(|l| l.starts_with("Snapshot")).or(lines.last()).cloned().unwrap_or_default().replace(['\t', '\n'], " ");
    let line = format!("{}\t{}\t{}", chrono::Local::now().format("%d.%m.%Y %H:%M"), match result {
        RunResult::Ok => "ok",
        RunResult::Failed => "fehler",
        RunResult::Cancelled => "abbruch",
    }, summary);
    let mut all: Vec<String> = std::fs::read_to_string(&p).unwrap_or_default().lines().map(String::from).collect();
    all.push(line);
    let skip = all.len().saturating_sub(HISTORY_MAX);
    write_atomic(&p, &(all[skip..].join("\n") + "\n"))?;
    Ok(())
}

/// Die letzten `n` Läufe, neueste zuerst.
pub fn read_history(n: usize) -> Vec<HistoryEntry> {
    let Ok(p) = history_path() else { return vec![] };
    let text = std::fs::read_to_string(p).unwrap_or_default();
    text.lines().rev().take(n).filter_map(parse_history_line).collect()
}

/// Schreibt das Protokoll des letzten Laufs (Zeitplan oder GUI) in `last-run.log` und ergänzt den Verlauf.
/// Ein Erfolg setzt auch die Fehlerzählung des Zeitplans zurück.
pub fn write_last_run(result: RunResult, lines: &[String]) -> Result<()> {
    let _ = append_history(result, lines);
    if result == RunResult::Ok {
        write_ts("last-success.txt", now());
        clear_schedule_failures();
    }
    let p = last_run_path()?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    let word = match result {
        RunResult::Ok => "ERFOLGREICH",
        RunResult::Failed => "FEHLER",
        RunResult::Cancelled => "ABGEBROCHEN",
    };
    let head = format!("{} – {word}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
    write_atomic(&p, &format!("{head}\n{}\n", lines.join("\n")))?;
    Ok(())
}

/// Liest das Protokoll des letzten Laufs (falls vorhanden).
pub fn read_last_run() -> Option<String> {
    std::fs::read_to_string(last_run_path().ok()?).ok()
}

impl Config {
    pub fn load() -> Result<Self> {
        let p = config_path()?;
        if !p.exists() {
            return Ok(Self::default());
        }
        let s = std::fs::read_to_string(&p).with_context(|| format!("Lesen von {}", p.display()))?;
        Self::parse(&s)
    }

    fn parse(s: &str) -> Result<Self> {
        let mut c: Self = toml::from_str(s)?;
        // Ältere Konfiguration mit nur einer Uhrzeit übernehmen
        if !s.contains("schedule_times") && !c.schedule_time.is_empty() {
            c.schedule_times = vec![c.schedule_time.clone()];
        }
        Ok(c)
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path()?;
        std::fs::create_dir_all(p.parent().unwrap())?;
        write_atomic(&p, &toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Schreibt die Einstellungen in eine Datei (ohne Passwörter, die liegen nur im Schlüsselbund).
    pub fn export_to(&self, path: &std::path::Path) -> Result<()> {
        std::fs::write(path, toml::to_string_pretty(self)?).with_context(|| format!("Schreiben von {}", path.display()))
    }

    /// Liest eine exportierte Konfiguration und prüft sie.
    pub fn import_from(path: &std::path::Path) -> Result<Self> {
        let s = std::fs::read_to_string(path).with_context(|| format!("Lesen von {}", path.display()))?;
        let c = Self::parse(&s).context("Datei ist keine gültige Konfiguration")?;
        c.validate()?;
        Ok(c)
    }

    /// Prüft die Eingaben auf Konsistenz.
    pub fn validate(&self) -> Result<()> {
        if self.schedule_times.is_empty() {
            anyhow::bail!("Mindestens eine Uhrzeit für den Zeitplan angeben");
        }
        for t in &self.schedule_times {
            crate::schedule::parse_time(t).with_context(|| format!("Ungültige Uhrzeit „{t}“ (HH:MM)"))?;
        }
        if !self.schedule_days.iter().any(|d| *d) {
            anyhow::bail!("Mindestens ein Wochentag muss für den Zeitplan gewählt sein");
        }
        if !(1..=100).contains(&self.check_subset_percent) {
            anyhow::bail!("Der Anteil für die Repo-Prüfung muss zwischen 1 und 100 Prozent liegen");
        }
        if self.keep_daily + self.keep_weekly + self.keep_monthly == 0 {
            anyhow::bail!("Mindestens eine Aufbewahrungsregel muss größer als 0 sein");
        }
        Ok(())
    }

    pub fn is_ready(&self) -> bool {
        !self.host.is_empty() && !self.share.is_empty() && !self.sources.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_failures_format_and_streak() {
        let t = |s: &str| chrono::NaiveDateTime::parse_from_str(s, TS_FMT).unwrap();
        assert_eq!(parse_schedule_failures("3\t2026-11-10 20:00:00\n"), Some((3, t("2026-11-10 20:00:00"))));
        assert_eq!(parse_schedule_failures("kaputt"), None);
        assert_eq!(parse_schedule_failures(""), None);
        // Folge: innerhalb von 6 Stunden zählt es weiter, danach von vorn
        assert_eq!(next_failure_count(None, t("2026-11-10 20:00:00")), 1);
        assert_eq!(next_failure_count(Some((2, t("2026-11-10 18:00:00"))), t("2026-11-10 20:00:00")), 3);
        assert_eq!(next_failure_count(Some((4, t("2026-11-09 20:00:00"))), t("2026-11-10 20:00:00")), 1);
    }

    #[test]
    fn schedule_log_dedupes_and_truncates() {
        let mut l = vec![];
        push_schedule_log(&mut l, "t1", "Kein Backup fällig");
        push_schedule_log(&mut l, "t2", "Kein Backup fällig");
        assert_eq!(l, vec!["t2\tKein Backup fällig"]);
        push_schedule_log(&mut l, "t3", "Backup\tgestartet");
        push_schedule_log(&mut l, "t4", "Kein Backup fällig");
        assert_eq!(l, vec!["t2\tKein Backup fällig", "t3\tBackup gestartet", "t4\tKein Backup fällig"]);
        for i in 0..500 {
            push_schedule_log(&mut l, "t", &format!("Meldung {i}"));
        }
        assert_eq!(l.len(), SCHEDULE_LOG_MAX);
        assert_eq!(l.last().unwrap(), "t\tMeldung 499");
    }

    #[test]
    fn history_line_parsed() {
        let h = parse_history_line("10.11.2026 20:00\tabbruch\tBackup abgebrochen.").unwrap();
        assert_eq!((h.time.as_str(), h.result, h.summary.as_str()), ("10.11.2026 20:00", RunResult::Cancelled, "Backup abgebrochen."));
        assert_eq!(parse_history_line("t\tok\tx").unwrap().result, RunResult::Ok);
        assert_eq!(parse_history_line("t\tfehler").unwrap().result, RunResult::Failed);
        assert!(parse_history_line("nur-zeit").is_none());
    }

    #[test]
    fn run_status_roundtrip_and_robust() {
        let t = chrono::NaiveDate::from_ymd_opt(2026, 11, 10).unwrap().and_hms_opt(16, 37, 5).unwrap();
        let s = RunStatus { started: Some(t), percent: 0.42, text: "Sichere\tDateien\n".into() };
        let back = parse_run_status(&format_run_status(&s));
        assert_eq!(back, RunStatus { started: Some(t), percent: 0.42, text: "Sichere Dateien".into() });
        // Kaputte oder leere Datei: keine Panik, Standardwerte
        assert_eq!(parse_run_status(""), RunStatus::default());
        assert_eq!(parse_run_status("müll"), RunStatus { text: String::new(), ..Default::default() });
        assert_eq!(parse_run_status("\t7.5\tx").percent, 1.0);
    }

    #[test]
    fn common_excludes_per_system() {
        let all = common_excludes();
        let mut seen = std::collections::HashSet::new();
        assert!(all.iter().all(|e| seen.insert(*e)), "doppelte Muster");
        assert!(all.contains(&"**/node_modules"));
        if cfg!(windows) {
            assert!(all.contains(&"**/AppData/Local/Temp"));
            assert!(!all.contains(&"**/.local/share/Trash"));
        } else {
            assert!(all.contains(&"**/.local/share/Trash"));
            assert!(!all.contains(&"**/AppData/Local/Temp"));
        }
        let (w, l): (std::collections::HashSet<_>, std::collections::HashSet<_>) = (COMMON_WINDOWS.iter().collect(), COMMON_LINUX.iter().collect());
        assert!(w.is_disjoint(&l));
    }

    #[test]
    fn add_excludes_splits_lines_and_dedups() {
        let mut l = vec!["*.tmp".to_string()];
        let n = add_excludes(&mut l, "**/node_modules\r\n\n# Kommentar\n  *.tmp  \n**/AppData/Local/Temp\n");
        assert_eq!(n, 2);
        assert_eq!(l, ["*.tmp", "**/node_modules", "**/AppData/Local/Temp"]);
    }

    #[test]
    fn roundtrip() {
        let mut c = Config::default();
        c.host = "nas".into();
        c.sources.push("/home/x".into());
        let s = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&s).unwrap(), c);
    }

    #[test]
    fn export_import_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("export.toml");
        let mut c = Config::default();
        c.host = "nas".into();
        c.sources = vec!["/a".into(), "/b".into()];
        c.export_to(&p).unwrap();
        assert_eq!(Config::import_from(&p).unwrap(), c);
        std::fs::write(&p, "das ist {kein toml").unwrap();
        assert!(Config::import_from(&p).is_err());
        std::fs::write(&p, "schedule_times = []").unwrap();
        assert!(Config::import_from(&p).is_err());
    }

    #[test]
    fn write_atomic_replaces_and_leaves_no_temp() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.txt");
        write_atomic(&p, "alt").unwrap();
        write_atomic(&p, "neu").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "neu");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    #[test]
    fn prune_due_rules() {
        let now = chrono::NaiveDateTime::parse_from_str("2026-11-06 12:00:00", CHECK_FMT).unwrap();
        let alt = parse_last_check("2026-10-30 12:00:00	ok
").unwrap();
        let neu = parse_last_check("2026-11-05 12:00:00	ok
").unwrap();
        assert!(prune_due(None, now));
        assert!(prune_due(Some(&alt), now));
        assert!(!prune_due(Some(&neu), now));
    }

    #[test]
    fn check_due_rules() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap().and_hms_opt(12, 0, 0).unwrap();
        let l = parse_last_check("2026-10-30 12:00:00\tok\n").unwrap();
        assert!(l.ok);
        assert!(!check_due(Some(&l), 7, now));
        assert!(check_due(Some(&l), 5, now));
        assert!(check_due(None, 7, now));
        assert!(!check_due(None, 0, now));
        assert!(!check_due(Some(&l), 0, now));
        assert_eq!(parse_last_check("kaputt"), None);
    }

    #[test]
    fn pause_rules() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 11, 5).unwrap().and_hms_opt(12, 0, 0).unwrap();
        assert!(!pause_active(None, now));
        assert!(pause_active(Some(now + chrono::Duration::hours(1)), now));
        assert!(!pause_active(Some(now), now));
        assert!(!pause_active(Some(now - chrono::Duration::days(1)), now));
    }

    #[test]
    fn validate_rules() {
        let mut c = Config::default();
        assert!(c.validate().is_ok());
        c.schedule_times = vec!["99:99".into()];
        assert!(c.validate().is_err());
        c.schedule_times = vec![];
        assert!(c.validate().is_err());
        c.schedule_times = vec!["02:00".into(), "14:30".into()];
        assert!(c.validate().is_ok());
        c.schedule_days = [false; 7];
        assert!(c.validate().is_err());
        c.schedule_days = [true; 7];
        c.keep_daily = 0;
        c.keep_weekly = 0;
        c.keep_monthly = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn old_single_time_is_migrated() {
        let c = Config::parse("schedule_time = \"04:15\"").unwrap();
        assert_eq!(c.schedule_times, vec!["04:15".to_string()]);
        let c = Config::parse("schedule_times = [\"01:00\", \"13:00\"]").unwrap();
        assert_eq!(c.schedule_times.len(), 2);
    }

    #[test]
    fn partial_file_uses_defaults() {
        let c: Config = toml::from_str("host = \"nas\"").unwrap();
        assert_eq!(c.keep_daily, 7);
    }
}
