use crate::config::Config;
use crate::restic::{BackupEvent, Restic};
use crate::secrets::{self, Secret};
use crate::smb;
use anyhow::{Context, Result};
use std::sync::atomic::AtomicBool;

/// Mountet die Freigabe und liefert einen Restic-Wrapper für das Repo.
pub fn open(cfg: &Config) -> Result<Restic> {
    let smb_pw = secrets::get(Secret::Smb)?.context("Kein SMB-Passwort gespeichert")?;
    let repo_pw = secrets::get(Secret::Repo)?.context("Kein Repo-Passwort gespeichert")?;
    smb::mount(cfg, &smb_pw)?;
    Ok(Restic { bin: cfg.restic_bin.clone(), repo: smb::repo_path(cfg)?, password: repo_pw })
}

/// Mount → (init) → Backup → Aufräumen → Unmount.
pub fn run_backup(cfg: &Config, keep_mounted: bool, cancel: &AtomicBool, mut log: impl FnMut(String), on_event: impl FnMut(BackupEvent)) -> Result<()> {
    log("Verbinde mit NAS …".into());
    let r = open(cfg)?;
    let result = (|| {
        if !r.exists() {
            log("Initialisiere neues Repository …".into());
            r.init()?;
        }
        log("Sichere Daten …".into());
        r.backup(&cfg.sources, &cfg.excludes, cancel, on_event)?;
        log("Räume alte Snapshots auf …".into());
        r.forget(cfg.keep_daily, cfg.keep_weekly, cfg.keep_monthly)?;
        Ok(())
    })();
    if keep_mounted {
        return result;
    }
    if let Err(e) = smb::unmount(cfg) {
        log(format!("Warnung: Unmount fehlgeschlagen: {e}"));
    }
    result
}
