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
    /// Uhrzeit (HH:MM) für das tägliche Backup
    pub schedule_time: String,
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
            schedule_time: "02:00".into(),
            restic_bin: default_restic_bin(),
        }
    }
}

/// Übliche Ausschlüsse als Vorlage für die GUI.
pub const COMMON_EXCLUDES: &[&str] = &[
    "node_modules", "target", ".cache", "*.tmp", "Thumbs.db", "$RECYCLE.BIN", "pagefile.sys", "hiberfil.sys", "System Volume Information",
];

/// Bevorzugt ein neben der Anwendung liegendes restic, sonst das aus dem PATH.
pub fn default_restic_bin() -> String {
    let name = if cfg!(windows) { "restic.exe" } else { "restic" };
    if let Ok(exe) = std::env::current_exe() {
        if let Some(p) = exe.parent().map(|d| d.join(name)) {
            if p.exists() {
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

/// Schreibt das Protokoll des letzten Laufs (Zeitplan oder GUI) in `last-run.log`.
pub fn write_last_run(ok: bool, lines: &[String]) -> Result<()> {
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
        Ok(toml::from_str(&s)?)
    }

    pub fn save(&self) -> Result<()> {
        let p = config_path()?;
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// Prüft die Eingaben auf Konsistenz.
    pub fn validate(&self) -> Result<()> {
        crate::schedule::parse_time(&self.schedule_time).context("Ungültige Uhrzeit (HH:MM)")?;
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
        c.schedule_time = "99:99".into();
        assert!(c.validate().is_err());
        c.schedule_time = "02:00".into();
        c.keep_daily = 0;
        c.keep_weekly = 0;
        c.keep_monthly = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn partial_file_uses_defaults() {
        let c: Config = toml::from_str("host = \"nas\"").unwrap();
        assert_eq!(c.keep_daily, 7);
    }
}
