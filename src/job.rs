use crate::config::Config;
use crate::restic::{BackupEvent, BackupOptions, BackupOutcome, RepoState, Restic, local_hostname};
use crate::secrets::{self, Secret};
use crate::smb;
use anyhow::{Context, Result, bail};
use std::sync::atomic::{AtomicBool, Ordering};

/// Gesetzt, wenn die App die Freigabe selbst eingebunden hat (nur dann wird sie auch wieder getrennt).
static MOUNTED_BY_APP: AtomicBool = AtomicBool::new(false);

/// Es läuft schon ein Backup (GUI oder Zeitplan); ein zweites würde das Repository sperren.
#[derive(Debug)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Es läuft bereits ein Backup.")
    }
}

impl std::error::Error for AlreadyRunning {}

/// Mountet die Freigabe und liefert einen Restic-Wrapper für das Repo.
pub fn open(cfg: &Config) -> Result<Restic> {
    let smb_pw = secrets::get(Secret::Smb)?.context("Kein SMB-Passwort gespeichert")?;
    let repo_pw = secrets::get(Secret::Repo)?.context("Kein Repo-Passwort gespeichert")?;
    if smb::mount(cfg, &smb_pw)? {
        MOUNTED_BY_APP.store(true, Ordering::Relaxed);
    }
    Ok(Restic { bin: cfg.restic_bin.clone(), repo: smb::repo_path(cfg)?, password: repo_pw })
}

/// Wie `open`, versucht es aber mehrfach (für Zeitplan-Läufe kurz nach dem Anmelden, wenn das
/// Netzwerk oder das NAS noch nicht erreichbar ist).
pub fn open_with_retry(cfg: &Config, attempts: u32, wait: std::time::Duration, mut log: impl FnMut(String)) -> Result<Restic> {
    let mut last = None;
    for i in 1..=attempts {
        match open(cfg) {
            Ok(r) => return Ok(r),
            Err(e) => {
                if i < attempts {
                    log(format!("Warnung: NAS nicht erreichbar (Versuch {i}/{attempts}): {e:#} – neuer Versuch in {} s", wait.as_secs()));
                    std::thread::sleep(wait);
                }
                last = Some(e);
            }
        }
    }
    Err(last.context("Keine Verbindungsversuche")?)
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

/// Entfernt Repository-Sperren. Mit `remove_all` auch aktive (Läufe anderer Rechner sind dadurch nicht geschützt).
pub fn unlock_repo(cfg: &Config, remove_all: bool) -> Result<String> {
    // Ein Backup auf diesem Rechner darf seine Sperre nicht verlieren
    let _lock = crate::config::RunLock::acquire()?.ok_or(AlreadyRunning)?;
    open(cfg)?.unlock(remove_all)?;
    Ok(if remove_all { "Alle Sperren entfernt." } else { "Veraltete Sperren entfernt." }.into())
}

/// Kennzahlen des Repos für die Übersicht.
#[derive(Debug, Clone)]
pub struct RepoOverview {
    pub size: u64,
    pub snapshots: usize,
    pub free: Option<u64>,
}

pub fn repo_overview(cfg: &Config) -> Result<RepoOverview> {
    let r = open(cfg)?;
    let stats = r.stats()?;
    Ok(RepoOverview { size: stats.size, snapshots: r.snapshots()?.len(), free: smb::free_space(&r.repo) })
}

/// Sichert die Quellordner: entweder alle in einem Snapshot oder (`separate_snapshots`) je Ordner einen.
/// Schlägt nur ein Teil der Ordner fehl, gilt der Lauf als Warnung; schlagen alle fehl, ist es ein Fehler.
fn backup_sources(r: &Restic, cfg: &Config, host: &str, cancel: &AtomicBool, mut log: impl FnMut(String), mut on_event: impl FnMut(BackupEvent)) -> Result<BackupOutcome> {
    let groups: Vec<&[String]> = if cfg.separate_snapshots { cfg.sources.chunks(1).collect() } else { vec![&cfg.sources[..]] };
    let mut outcome = BackupOutcome::default();
    let mut errors = vec![];
    let (pack_size_mib, connections) = crate::restic::tuning_for(r.version());
    for group in &groups {
        if cancel.load(Ordering::Relaxed) {
            bail!("Backup abgebrochen");
        }
        log(if cfg.separate_snapshots { format!("Sichere {} …", group[0]) } else { "Sichere Daten …".into() });
        let res = r.backup(
            &BackupOptions {
                sources: group,
                excludes: &cfg.excludes,
                exclude_caches: cfg.exclude_caches,
                exclude_if_present: &cfg.exclude_if_present,
                exclude_larger_than: &cfg.exclude_larger_than,
                host,
                no_scan: cfg.no_scan,
                pack_size_mib,
                connections,
            },
            cancel,
            &mut on_event,
        );
        match res {
            Ok(o) => outcome.warnings |= o.warnings,
            Err(e) if cancel.load(Ordering::Relaxed) => return Err(e),
            Err(e) => {
                log(format!("Fehler bei {}: {e:#}", group.join(", ")));
                errors.push(format!("{}: {e:#}", group.join(", ")));
            }
        }
    }
    if errors.len() == groups.len() {
        bail!("{}", errors.join("\n"));
    }
    if !errors.is_empty() {
        outcome.warnings = true;
        log(format!("Warnung: {} von {} Ordnern konnten nicht gesichert werden.", errors.len(), groups.len()));
    }
    Ok(outcome)
}

/// Mount → (init) → Backup → Aufräumen → Unmount.
pub fn run_backup(cfg: &Config, keep_mounted: bool, cancel: &AtomicBool, mut log: impl FnMut(String), on_event: impl FnMut(BackupEvent)) -> Result<BackupOutcome> {
    cfg.validate()?;
    if !cfg.is_ready() {
        bail!("Einstellungen unvollständig: NAS-Adresse, Freigabe und mindestens ein Quellordner sind nötig");
    }
    let _lock = crate::config::RunLock::acquire()?.ok_or(AlreadyRunning)?;
    log("Verbinde mit NAS …".into());
    // Zeitplan-Lauf (headless): 2 Versuche à 30 s (die Erreichbarkeit wurde vorher geprüft), GUI-Lauf: sofort melden
    let r = if keep_mounted { open(cfg)? } else { open_with_retry(cfg, 2, std::time::Duration::from_secs(30), &mut log)? };
    let host = if cfg.backup_host.trim().is_empty() { local_hostname() } else { cfg.backup_host.trim().to_string() };
    let result = (|| {
        // Schneller Zugriffstest: restic würde bei verweigertem Zugriff (falsche Anmeldung) minutenlang
        // wiederholen; ein fehlender Ordner ist dagegen Sache von `probe` bzw. „Neues Repo anlegen“.
        if let Err(e) = std::fs::read_dir(&r.repo) {
            if e.kind() != std::io::ErrorKind::NotFound {
                bail!("Kein Zugriff auf das Repository ({}): {e}", r.repo);
            }
        }
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
        let outcome = backup_sources(&r, cfg, &host, cancel, &mut log, on_event)?;
        let mut outcome = outcome;
        // Der Snapshot ist schon geschrieben: scheitert nur das Aufräumen (z. B. Repo von einem anderen Rechner
        // gesperrt), ist das eine Warnung. Als Fehler würde der Zeitplan alle 30 Minuten neu sichern.
        log("Räume alte Snapshots auf …".into());
        // Prune (Index und Trees über SMB lesen, Packs umschreiben) ist langsam: nur alle paar Tage
        if host.is_empty() {
            // Ohne Rechnernamen würde forget die Snapshots aller Rechner im Repo ausdünnen
            log("Warnung: Rechnername unbekannt – Aufräumen übersprungen".into());
            outcome.warnings = true;
            return Ok(outcome);
        }
        let prune = crate::config::prune_due(crate::config::read_last_prune().as_ref(), chrono::Local::now().naive_local());
        match r.forget(cfg.keep_daily, cfg.keep_weekly, cfg.keep_monthly, &host, prune) {
            Ok(()) => {
                if prune {
                    crate::config::write_last_prune();
                }
            }
            Err(e) => {
                log(format!("Warnung: Aufräumen fehlgeschlagen: {e:#}"));
                outcome.warnings = true;
            }
        }
        // Regelmäßige Teilprüfung nur im Zeitplan-Lauf; ein Fehler dort ist eine Warnung, kein Backup-Fehler
        if !keep_mounted && crate::config::check_due(crate::config::read_last_check().as_ref(), cfg.check_interval_days, chrono::Local::now().naive_local()) {
            log(format!("Prüfe Repository ({} % der Daten) …", cfg.check_subset_percent));
            match r.check_subset(cfg.check_subset_percent) {
                Ok(()) => {
                    crate::config::write_last_check(true);
                    log("Repository-Prüfung bestanden.".into());
                }
                Err(e) => {
                    crate::config::write_last_check(false);
                    log(format!("Warnung: Repository-Prüfung fehlgeschlagen: {e:#}"));
                    outcome.warnings = true;
                }
            }
        }
        Ok(outcome)
    })();
    if !keep_mounted {
        if let Err(e) = unmount_if_ours(cfg) {
            log(format!("Warnung: Unmount fehlgeschlagen: {e}"));
        }
    }
    result.map_err(|e| {
        if format!("{e:#}").contains("locked") {
            e.context("Das Repository ist gesperrt. Im Tab „Snapshots“ lassen sich die Sperren mit „Sperren entfernen“ löschen")
        } else {
            e
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn restic_available() -> bool {
        std::process::Command::new("restic").arg("version").output().is_ok()
    }

    fn setup(d: &std::path::Path, names: &[&str]) -> (Restic, Config) {
        let mut cfg = Config::default();
        for n in names {
            let p = d.join(n);
            std::fs::create_dir(&p).unwrap();
            std::fs::write(p.join("a.txt"), n).unwrap();
            cfg.sources.push(p.to_string_lossy().into());
        }
        let r = Restic { bin: "restic".into(), repo: d.join("repo").to_string_lossy().into(), password: "pw".into() };
        r.init().unwrap();
        (r, cfg)
    }

    #[test]
    fn separate_snapshots_one_per_folder() {
        if !restic_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let (r, mut cfg) = setup(d.path(), &["admin", "xyz"]);
        cfg.separate_snapshots = true;
        backup_sources(&r, &cfg, "testhost", &AtomicBool::new(false), |_| {}, |_| {}).unwrap();
        let snaps = r.snapshots().unwrap();
        assert_eq!(snaps.len(), 2);
        assert!(snaps.iter().all(|s| s.paths.len() == 1));
        // Aufbewahrung gilt je Ordner: beide Snapshots bleiben erhalten
        r.forget(1, 0, 0, "testhost", true).unwrap();
        assert_eq!(r.snapshots().unwrap().len(), 2);
    }

    #[test]
    fn combined_snapshot_by_default() {
        if !restic_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let (r, cfg) = setup(d.path(), &["admin", "xyz"]);
        backup_sources(&r, &cfg, "testhost", &AtomicBool::new(false), |_| {}, |_| {}).unwrap();
        let snaps = r.snapshots().unwrap();
        assert_eq!(snaps.len(), 1);
        assert_eq!(snaps[0].paths.len(), 2);
    }

    #[test]
    fn partial_failure_is_warning_total_failure_is_error() {
        if !restic_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let (r, mut cfg) = setup(d.path(), &["admin"]);
        cfg.separate_snapshots = true;
        cfg.sources.push(d.path().join("gibt-es-nicht").to_string_lossy().into());
        let mut lines = vec![];
        let o = backup_sources(&r, &cfg, "testhost", &AtomicBool::new(false), |m| lines.push(m), |_| {}).unwrap();
        assert!(o.warnings);
        assert_eq!(r.snapshots().unwrap().len(), 1);
        cfg.sources.remove(0);
        assert!(backup_sources(&r, &cfg, "testhost", &AtomicBool::new(false), |_| {}, |_| {}).is_err());
    }

    #[test]
    fn stats_diff_and_check_subset_work() {
        if !restic_available() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let (r, mut cfg) = setup(d.path(), &["admin"]);
        cfg.separate_snapshots = true;
        backup_sources(&r, &cfg, "testhost", &AtomicBool::new(false), |_| {}, |_| {}).unwrap();
        std::fs::write(d.path().join("admin").join("neu.txt"), "neu").unwrap();
        backup_sources(&r, &cfg, "testhost", &AtomicBool::new(false), |_| {}, |_| {}).unwrap();
        let snaps = r.snapshots().unwrap();
        assert_eq!(snaps.len(), 2);
        assert!(r.stats().unwrap().size > 0);
        r.check_subset(50).unwrap();
        let diff = r.diff(&snaps[0].id, &snaps[1].id).unwrap();
        assert!(diff.iter().any(|e| e.modifier == "+" && e.path.ends_with("neu.txt")));
    }
}
