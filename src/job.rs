use crate::config::Config;
use crate::restic::{BackupEvent, BackupOptions, BackupOutcome, RepoState, Restic, local_hostname};
use crate::secrets::{self, Secret};
use crate::smb;
use anyhow::{Context, Result, bail};
use std::sync::atomic::{AtomicBool, Ordering};

/// Gesetzt, wenn die App die Freigabe selbst eingebunden hat (nur dann wird sie auch wieder getrennt).
static MOUNTED_BY_APP: AtomicBool = AtomicBool::new(false);

/// Mountet die Freigabe und liefert einen Restic-Wrapper für das Repo.
pub fn open(cfg: &Config) -> Result<Restic> {
    let smb_pw = secrets::get(Secret::Smb)?.context("Kein SMB-Passwort gespeichert")?;
    let repo_pw = secrets::get(Secret::Repo)?.context("Kein Repo-Passwort gespeichert")?;
    if smb::mount(cfg, &smb_pw)? {
        MOUNTED_BY_APP.store(true, Ordering::Relaxed);
    }
    Ok(Restic { bin: cfg.restic_bin.clone(), repo: smb::repo_path(cfg)?, password: repo_pw })
}

/// Trennt die Freigabe, falls die App sie eingebunden hat.
pub fn unmount_if_ours(cfg: &Config) -> Result<()> {
    if MOUNTED_BY_APP.swap(false, Ordering::Relaxed) {
        smb::unmount(cfg)?;
    }
    Ok(())
}

/// Beschreibung des Repo-Zustands für die Anzeige.
pub fn describe_repo(cfg: &Config) -> Result<String> {
    let r = open(cfg)?;
    Ok(match r.probe() {
        RepoState::Ok => format!("Repository vorhanden, {} Snapshot(s).", r.snapshots()?.len()),
        RepoState::NotFound => "Am Pfad existiert kein Repository.".into(),
        RepoState::WrongPassword => "Repository gefunden, aber das Passwort ist falsch.".into(),
        RepoState::Error(e) => format!("Fehler beim Zugriff: {e}"),
    })
}

/// Mount → (init) → Backup → Aufräumen → Unmount.
pub fn run_backup(cfg: &Config, keep_mounted: bool, cancel: &AtomicBool, mut log: impl FnMut(String), on_event: impl FnMut(BackupEvent)) -> Result<BackupOutcome> {
    cfg.validate()?;
    log("Verbinde mit NAS …".into());
    let r = open(cfg)?;
    let host = local_hostname();
    let result = (|| {
        match r.probe() {
            RepoState::Ok => {}
            RepoState::NotFound if cfg.allow_init => {
                log("Initialisiere neues Repository …".into());
                r.init()?;
            }
            RepoState::NotFound => bail!("Am Repo-Pfad existiert kein Repository. Unterordner prüfen oder „Neues Repo anlegen“ in den Einstellungen aktivieren."),
            RepoState::WrongPassword => bail!("Repo-Passwort ist falsch"),
            RepoState::Error(e) => bail!("Repository nicht erreichbar: {e}"),
        }
        log("Sichere Daten …".into());
        let outcome = r.backup(
            &BackupOptions {
                sources: &cfg.sources,
                excludes: &cfg.excludes,
                exclude_caches: cfg.exclude_caches,
                exclude_if_present: &cfg.exclude_if_present,
                exclude_larger_than: &cfg.exclude_larger_than,
                host: &host,
            },
            cancel,
            on_event,
        )?;
        if outcome.warnings {
            log("Warnung: Einige Dateien waren nicht lesbar und wurden übersprungen.".into());
        }
        log("Räume alte Snapshots auf …".into());
        r.forget(cfg.keep_daily, cfg.keep_weekly, cfg.keep_monthly, &host)?;
        Ok(outcome)
    })();
    if !keep_mounted {
        if let Err(e) = unmount_if_ours(cfg) {
            log(format!("Warnung: Unmount fehlgeschlagen: {e}"));
        }
    }
    result
}
