use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Restic {
    pub bin: String,
    pub repo: String,
    pub password: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Snapshot {
    pub id: String,
    #[serde(default)]
    pub short_id: String,
    pub time: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Node {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Zähler für spätere Statusanzeige
pub enum BackupEvent {
    Progress { percent: f64, files_done: u64, total_files: u64, bytes_done: u64, total_bytes: u64 },
    Summary { snapshot_id: String, files_new: u64, data_added: u64 },
}

#[derive(Deserialize)]
struct RawEvent {
    message_type: String,
    #[serde(default)]
    percent_done: f64,
    #[serde(default)]
    files_done: u64,
    #[serde(default)]
    total_files: u64,
    #[serde(default)]
    bytes_done: u64,
    #[serde(default)]
    total_bytes: u64,
    #[serde(default)]
    snapshot_id: String,
    #[serde(default)]
    files_new: u64,
    #[serde(default)]
    data_added: u64,
}

pub fn parse_backup_line(line: &str) -> Option<BackupEvent> {
    let r: RawEvent = serde_json::from_str(line).ok()?;
    match r.message_type.as_str() {
        "status" => Some(BackupEvent::Progress {
            percent: r.percent_done,
            files_done: r.files_done,
            total_files: r.total_files,
            bytes_done: r.bytes_done,
            total_bytes: r.total_bytes,
        }),
        "summary" => Some(BackupEvent::Summary {
            snapshot_id: r.snapshot_id,
            files_new: r.files_new,
            data_added: r.data_added,
        }),
        _ => None,
    }
}

pub fn parse_ls(output: &str) -> Vec<Node> {
    output
        .lines()
        .filter_map(|l| serde_json::from_str::<Node>(l).ok())
        .collect()
}

pub fn forget_args(daily: u32, weekly: u32, monthly: u32) -> Vec<String> {
    let mut a = vec!["forget".to_string(), "--prune".into()];
    for (flag, n) in [("--keep-daily", daily), ("--keep-weekly", weekly), ("--keep-monthly", monthly)] {
        if n > 0 {
            a.push(flag.into());
            a.push(n.to_string());
        }
    }
    a
}

impl Restic {
    fn cmd(&self, args: &[String]) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(args)
            .env("RESTIC_REPOSITORY", &self.repo)
            .env("RESTIC_PASSWORD", &self.password)
            .stdin(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        c
    }

    fn run(&self, args: &[String]) -> Result<String> {
        let out = self
            .cmd(args)
            .output()
            .with_context(|| format!("'{}' konnte nicht gestartet werden", self.bin))?;
        if !out.status.success() {
            bail!("restic {} fehlgeschlagen: {}", args.first().map(String::as_str).unwrap_or(""), String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    /// Prüft, ob am Repo-Pfad bereits ein Repository existiert.
    pub fn exists(&self) -> bool {
        self.run(&Self::s(&["cat", "config"])).is_ok()
    }

    pub fn init(&self) -> Result<()> {
        self.run(&Self::s(&["init"])).map(|_| ())
    }

    pub fn snapshots(&self) -> Result<Vec<Snapshot>> {
        let out = self.run(&Self::s(&["snapshots", "--json"]))?;
        Ok(serde_json::from_str(&out)?)
    }

    pub fn ls(&self, snapshot: &str, dir: &str) -> Result<Vec<Node>> {
        let out = self.run(&["ls".into(), "--json".into(), snapshot.into(), dir.into()])?;
        Ok(parse_ls(&out))
    }

    pub fn restore(&self, snapshot: &str, target: &str, include: Option<&str>) -> Result<()> {
        let mut a = Self::s(&["restore", snapshot, "--target", target]);
        if let Some(i) = include {
            a.push("--include".into());
            a.push(i.into());
        }
        self.run(&a).map(|_| ())
    }

    pub fn forget(&self, daily: u32, weekly: u32, monthly: u32) -> Result<()> {
        if daily + weekly + monthly == 0 {
            bail!("Keine Aufbewahrungsregel gesetzt – forget würde nichts behalten");
        }
        self.run(&forget_args(daily, weekly, monthly)).map(|_| ())
    }

    #[allow(dead_code)]
    pub fn check(&self) -> Result<()> {
        self.run(&Self::s(&["check"])).map(|_| ())
    }

    /// Führt ein Backup aus. Exit-Code 3 (einige Dateien nicht lesbar) gilt als Warnung, nicht als Fehler.
    pub fn backup(&self, sources: &[String], excludes: &[String], cancel: &AtomicBool, mut on_event: impl FnMut(BackupEvent)) -> Result<()> {
        let mut args = Self::s(&["backup", "--json"]);
        for e in excludes {
            args.push("--exclude".into());
            args.push(e.clone());
        }
        args.extend(sources.iter().cloned());
        let mut child = self
            .cmd(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("'{}' konnte nicht gestartet werden", self.bin))?;
        let stdout = child.stdout.take().unwrap();
        // stderr parallel einlesen, damit die Pipe nicht blockiert
        let stderr = child.stderr.take().unwrap();
        let err_thread = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
            s
        });
        let child = Arc::new(Mutex::new(child));
        let done = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        std::thread::scope(|sc| {
            // Wächter: beendet den restic-Prozess, sobald abgebrochen wird
            sc.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if cancel.load(Ordering::Relaxed) {
                        cancelled.store(true, Ordering::Relaxed);
                        let _ = child.lock().unwrap().kill();
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
            });
            for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
                if let Some(ev) = parse_backup_line(&line) {
                    on_event(ev);
                }
            }
            done.store(true, Ordering::Relaxed);
        });
        let status = child.lock().unwrap().wait()?;
        if cancelled.load(Ordering::Relaxed) {
            bail!("Backup abgebrochen");
        }
        let err = err_thread.join().unwrap_or_default();
        match status.code() {
            Some(0) | Some(3) => Ok(()),
            _ => bail!("restic backup fehlgeschlagen: {}", err.trim()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_summary() {
        let st = r#"{"message_type":"status","percent_done":0.5,"total_files":10,"files_done":5,"total_bytes":100,"bytes_done":50}"#;
        assert!(matches!(parse_backup_line(st), Some(BackupEvent::Progress { files_done: 5, .. })));
        let su = r#"{"message_type":"summary","snapshot_id":"abc","files_new":3,"data_added":42}"#;
        assert!(matches!(parse_backup_line(su), Some(BackupEvent::Summary { files_new: 3, .. })));
        assert!(parse_backup_line("kein json").is_none());
    }

    #[test]
    fn ls_skips_header() {
        let o = "{\"struct_type\":\"snapshot\",\"id\":\"x\"}\n{\"name\":\"a\",\"type\":\"file\",\"path\":\"/a\",\"size\":3}\n";
        let n = parse_ls(o);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].size, 3);
    }

    #[test]
    fn forget_skips_zero() {
        assert_eq!(forget_args(7, 0, 6), ["forget", "--prune", "--keep-daily", "7", "--keep-monthly", "6"]);
    }

    #[test]
    fn cancel_aborts_backup() {
        if Command::new("restic").arg("version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.txt"), "x").unwrap();
        let r = Restic { bin: "restic".into(), repo: d.path().join("repo").to_string_lossy().into(), password: "pw".into() };
        r.init().unwrap();
        let err = r.backup(&[src.to_string_lossy().into()], &[], &AtomicBool::new(true), |_| {});
        // Entweder rechtzeitig abgebrochen oder (bei sehr kleinem Backup) schon fertig – kein Hänger, kein Panic
        if let Err(e) = err {
            assert!(e.to_string().contains("abgebrochen"));
        }
    }

    #[test]
    fn wrong_password_is_error() {
        if Command::new("restic").arg("version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let mut r = Restic { bin: "restic".into(), repo: d.path().join("repo").to_string_lossy().into(), password: "pw".into() };
        r.init().unwrap();
        r.password = "falsch".into();
        assert!(!r.exists());
        assert!(r.snapshots().is_err());
    }

    #[test]
    fn full_cycle_local_repo() {
        if Command::new("restic").arg("version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.txt"), "hallo").unwrap();
        let r = Restic { bin: "restic".into(), repo: d.path().join("repo").to_string_lossy().into(), password: "pw".into() };
        assert!(!r.exists());
        r.init().unwrap();
        assert!(r.exists());
        let mut got_summary = false;
        r.backup(&[src.to_string_lossy().into()], &[], &AtomicBool::new(false), |e| got_summary |= matches!(e, BackupEvent::Summary { .. })).unwrap();
        assert!(got_summary);
        let snaps = r.snapshots().unwrap();
        assert_eq!(snaps.len(), 1);
        let out = d.path().join("out");
        r.restore(&snaps[0].id, &out.to_string_lossy(), None).unwrap();
        // restic stellt den vollen Originalpfad unter dem Ziel wieder her (Windows: Laufwerksbuchstabe als Ordner)
        let mut restored = out.clone();
        for c in src.components() {
            match c {
                std::path::Component::Prefix(p) => restored.push(p.as_os_str().to_string_lossy().replace(':', "")),
                std::path::Component::RootDir => {}
                other => restored.push(other),
            }
        }
        let restored = restored.join("a.txt");
        assert_eq!(std::fs::read_to_string(restored).unwrap(), "hallo");
        r.forget(1, 0, 0).unwrap();
        r.check().unwrap();
    }
}
