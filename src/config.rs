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
    pub excludes: Vec<String>,
    /// Verzeichnisse mit CACHEDIR.TAG ausschließen
    pub exclude_caches: bool,
    /// Ordner, die eine dieser Dateien enthalten, werden ausgeschlossen (z. B. `.nobackup`)
    pub exclude_if_present: Vec<String>,
    /// Dateien größer als dieser Wert werden ausgeschlossen (restic-Format, z. B. `500M`); leer = aus
    pub exclude_larger_than: String,
    /// Neues Repository anlegen, falls am Pfad keins existiert
    pub allow_init: bool,
    pub keep_daily: u32,
    pub keep_weekly: u32,
    pub keep_monthly: u32,
    /// Uhrzeiten (HH:MM), zu denen das Backup läuft
    pub schedule_times: Vec<String>,
    /// Wochentage (Mo..So), an denen der Zeitplan gilt
    pub schedule_days: [bool; 7],
    /// Alte Einstellung (nur zum Übernehmen älterer Konfigurationen)
    #[serde(skip_serializing)]
    schedule_time: String,
    pub restic_bin: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: String::new(),
            share: String::new(),
            subpath: "restic-repo".into(),
            smb_user: String::new(),
            sources: vec![],
            excludes: vec![],
            exclude_caches: false,
            exclude_if_present: vec![],
            exclude_larger_than: String::new(),
            allow_init: false,
            keep_daily: 7,
            keep_weekly: 4,
            keep_monthly: 6,
            schedule_times: vec!["02:00".into()],
            schedule_days: [true; 7],
            schedule_time: String::new(),
            restic_bin: default_restic_bin(),
        }
    }
}

/// Übliche Ausschlüsse als Vorlage für die GUI.
pub const COMMON_EXCLUDES: &[&str] = &[
    "node_modules", "target", ".cache", "*.tmp", "Thumbs.db", "$RECYCLE.BIN", "pagefile.sys", "hiberfil.sys", "System Volume Information",
];

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

/// Ein Eintrag im lokalen Verlauf.
pub struct HistoryEntry {
    pub time: String,
    pub ok: bool,
    pub summary: String,
}

const HISTORY_MAX: usize = 200;

fn append_history(ok: bool, lines: &[String]) -> Result<()> {
    let p = history_path()?;
    let summary = lines.iter().rev().find(|l| l.starts_with("Snapshot")).or(lines.last()).cloned().unwrap_or_default().replace(['\t', '\n'], " ");
    let line = format!("{}\t{}\t{}", chrono::Local::now().format("%d.%m.%Y %H:%M"), if ok { "ok" } else { "fehler" }, summary);
    let mut all: Vec<String> = std::fs::read_to_string(&p).unwrap_or_default().lines().map(String::from).collect();
    all.push(line);
    let skip = all.len().saturating_sub(HISTORY_MAX);
    std::fs::write(&p, all[skip..].join("\n") + "\n")?;
    Ok(())
}

/// Die letzten `n` Läufe, neueste zuerst.
pub fn read_history(n: usize) -> Vec<HistoryEntry> {
    let Ok(p) = history_path() else { return vec![] };
    let text = std::fs::read_to_string(p).unwrap_or_default();
    text.lines()
        .rev()
        .take(n)
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            Some(HistoryEntry { time: it.next()?.into(), ok: it.next()? == "ok", summary: it.next().unwrap_or("").into() })
        })
        .collect()
}

/// Schreibt das Protokoll des letzten Laufs (Zeitplan oder GUI) in `last-run.log` und ergänzt den Verlauf.
pub fn write_last_run(ok: bool, lines: &[String]) -> Result<()> {
    let _ = append_history(ok, lines);
    let p = last_run_path()?;
    std::fs::create_dir_all(p.parent().unwrap())?;
    let head = format!("{} – {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"), if ok { "ERFOLGREICH" } else { "FEHLER" });
    std::fs::write(&p, format!("{head}\n{}\n", lines.join("\n")))?;
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
        std::fs::write(&p, toml::to_string_pretty(self)?)?;
        Ok(())
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
    fn roundtrip() {
        let mut c = Config::default();
        c.host = "nas".into();
        c.sources.push("/home/x".into());
        let s = toml::to_string_pretty(&c).unwrap();
        assert_eq!(toml::from_str::<Config>(&s).unwrap(), c);
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
