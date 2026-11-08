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

#[derive(Debug, Clone, PartialEq)]
pub enum RepoState {
    Ok,
    NotFound,
    WrongPassword,
    Error(String),
}

/// Ergebnis eines Backups; `warnings` = einige Dateien waren nicht lesbar (restic-Exit-Code 3).
#[derive(Debug, Clone, Copy, Default)]
pub struct BackupOutcome {
    pub warnings: bool,
}

/// Rechnername, so wie restic ihn als Standard-Host verwendet.
pub fn local_hostname() -> String {
    let mut c = Command::new("hostname");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_default()
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

/// Größe des Repos (`restic stats --mode raw-data`).
#[derive(Debug, Clone, PartialEq)]
pub struct RepoStats {
    pub size: u64,
}

pub fn parse_stats(output: &str) -> Option<RepoStats> {
    #[derive(Deserialize)]
    struct Raw {
        total_size: u64,
    }
    Some(RepoStats { size: serde_json::from_str::<Raw>(output.trim()).ok()?.total_size })
}

/// Eine Änderung zwischen zwei Snapshots (`modifier`: `+` neu, `-` entfernt, `M` geändert, sonst z. B. `U`/`T`).
#[derive(Debug, Clone, PartialEq)]
pub struct DiffEntry {
    pub modifier: String,
    pub path: String,
}

pub fn parse_diff(output: &str) -> Vec<DiffEntry> {
    #[derive(Deserialize)]
    struct Raw {
        message_type: String,
        #[serde(default)]
        path: String,
        #[serde(default)]
        modifier: String,
    }
    output
        .lines()
        .filter_map(|l| serde_json::from_str::<Raw>(l).ok())
        .filter(|r| r.message_type == "change")
        .map(|r| DiffEntry { modifier: r.modifier, path: r.path })
        .collect()
}

pub fn parse_ls(output: &str) -> Vec<Node> {
    output
        .lines()
        .filter_map(|l| serde_json::from_str::<Node>(l).ok())
        .collect()
}

/// Suchmuster für `restic find`: ohne Platzhalter wird der Begriff in `*` gefasst (Teilstring im Namen).
pub fn find_pattern(term: &str) -> String {
    let t = term.trim();
    if t.contains('*') || t.contains('?') { t.to_string() } else { format!("*{t}*") }
}

/// Ausgabe von `restic find --json`: Treffer aller Snapshots als `Node`s (Name = letzter Pfadteil), höchstens `limit`.
pub fn parse_find(output: &str, limit: usize) -> Vec<Node> {
    #[derive(Deserialize)]
    struct Group {
        #[serde(default)]
        matches: Vec<Hit>,
    }
    #[derive(Deserialize)]
    struct Hit {
        path: String,
        #[serde(rename = "type", default)]
        kind: String,
        #[serde(default)]
        size: u64,
    }
    let groups: Vec<Group> = serde_json::from_str(output).unwrap_or_default();
    groups
        .into_iter()
        .flat_map(|g| g.matches)
        .take(limit)
        .map(|h| Node { name: h.path.rsplit('/').next().unwrap_or(&h.path).to_string(), path: h.path, kind: h.kind, size: h.size })
        .collect()
}

/// Optionen, die bestimmen, was ins Backup kommt.
#[derive(Default, Clone)]
pub struct BackupOptions<'a> {
    pub sources: &'a [String],
    pub excludes: &'a [String],
    pub exclude_caches: bool,
    pub exclude_if_present: &'a [String],
    pub exclude_larger_than: &'a str,
    /// Keine Vorabzählung der Dateien (`--no-scan`)
    pub no_scan: bool,
    pub host: &'a str,
    /// Ziel-Packgröße in MiB (0 = restic-Standard); größere Packs bedeuten weniger SMB-Roundtrips
    pub pack_size_mib: u32,
    /// Parallele Schreibvorgänge im Repo (0 = restic-Standard 2); auf SMB mit Latenz deutlich schneller
    pub connections: u32,
}

/// Version aus der Ausgabe von `restic version` („restic 0.18.1 compiled with …“).
pub fn parse_version(out: &str) -> Option<(u32, u32)> {
    let v = out.split_whitespace().nth(1)?;
    let mut it = v.split('.');
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// Einstellungen zur Beschleunigung, die es erst in neueren restic-Versionen gibt (`--pack-size` ab 0.14,
/// `-o local.connections` ab 0.17); ältere Versionen würden sonst mit „unknown flag/option“ scheitern.
pub fn tuning_for(version: Option<(u32, u32)>) -> (u32, u32) {
    match version {
        Some(v) if v >= (0, 17) => (64, 8),
        Some(v) if v >= (0, 14) => (64, 0),
        _ => (0, 0),
    }
}

pub fn backup_args(o: &BackupOptions) -> Vec<String> {
    // Mehr parallele Dateilesevorgänge beschleunigen das Einlesen vieler kleiner Dateien (restic ≥ 0.13)
    let mut a = vec!["backup".to_string(), "--json".into(), "--read-concurrency".into(), "4".into()];
    if !o.host.is_empty() {
        a.extend(["--host".into(), o.host.into()]);
    }
    for e in o.excludes {
        a.extend(["--exclude".into(), e.clone()]);
    }
    if o.exclude_caches {
        a.push("--exclude-caches".into());
    }
    for f in o.exclude_if_present.iter().filter(|f| !f.trim().is_empty()) {
        a.extend(["--exclude-if-present".into(), f.trim().into()]);
    }
    if !o.exclude_larger_than.trim().is_empty() {
        a.extend(["--exclude-larger-than".into(), o.exclude_larger_than.trim().into()]);
    }
    if o.no_scan {
        a.push("--no-scan".into());
    }
    if o.pack_size_mib > 0 {
        a.extend(["--pack-size".into(), o.pack_size_mib.to_string()]);
    }
    if o.connections > 0 {
        a.extend(["-o".into(), format!("local.connections={}", o.connections)]);
    }
    a.extend(o.sources.iter().cloned());
    a
}

/// `host`: nur Snapshots dieses Rechners aufräumen (wichtig bei geteilten Repos).
/// `prune`: zusätzlich ungenutzte Daten entfernen (langsam über SMB, daher nur gelegentlich).
pub fn forget_args(daily: u32, weekly: u32, monthly: u32, host: &str, prune: bool) -> Vec<String> {
    let mut a = vec!["forget".to_string()];
    if prune {
        a.extend(["--prune".into(), "--max-unused".into(), "10%".into()]);
    }
    if !host.is_empty() {
        a.extend(["--host".into(), host.into()]);
    }
    for (flag, n) in [("--keep-daily", daily), ("--keep-weekly", weekly), ("--keep-monthly", monthly)] {
        if n > 0 {
            a.push(flag.into());
            a.push(n.to_string());
        }
    }
    a
}

/// `remove_all`: auch Sperren entfernen, die noch aktiv aussehen (nur, wenn sicher kein anderer Lauf aktiv ist).
pub fn unlock_args(remove_all: bool) -> Vec<String> {
    let mut a = vec!["unlock".to_string()];
    if remove_all {
        a.push("--remove-all".into());
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
        // Fester Cache im Datenverzeichnis: GUI und Zeitplan (evtl. anderes Konto) nutzen denselben Cache,
        // sonst liest restic die Metadaten bei jedem Lauf neu über SMB
        if let Ok(d) = crate::config::data_dir() {
            c.env("RESTIC_CACHE_DIR", d.join("restic-cache"));
        }
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

    /// Prüft den Zustand des Repos (Exit 10 = nicht vorhanden, 12 = falsches Passwort).
    pub fn probe(&self) -> RepoState {
        match self.cmd(&Self::s(&["cat", "config"])).output() {
            Err(e) => RepoState::Error(format!("'{}' konnte nicht gestartet werden: {e}", self.bin)),
            Ok(o) if o.status.success() => RepoState::Ok,
            Ok(o) => match o.status.code() {
                Some(10) => RepoState::NotFound,
                Some(12) => RepoState::WrongPassword,
                _ => {
                    let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                    // Ältere restic-Versionen kennen die Exit-Codes nicht
                    if err.contains("wrong password") {
                        RepoState::WrongPassword
                    } else if err.contains("does not exist") || err.contains("Is there a repository at") {
                        RepoState::NotFound
                    } else {
                        RepoState::Error(err)
                    }
                }
            },
        }
    }

    /// Version des restic-Programms (Major, Minor); `None`, wenn sie sich nicht ermitteln lässt.
    pub fn version(&self) -> Option<(u32, u32)> {
        parse_version(&self.run(&Self::s(&["version"])).ok()?)
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

    /// Sucht Dateien und Ordner nach Namen in einem Snapshot (`restic find`, ohne Beachtung der Groß-/Kleinschreibung).
    pub fn find(&self, snapshot: &str, term: &str, limit: usize) -> Result<Vec<Node>> {
        let out = self.run(&["find".into(), "--json".into(), "--ignore-case".into(), "--snapshot".into(), snapshot.into(), find_pattern(term)])?;
        Ok(parse_find(&out, limit))
    }

    pub fn restore(&self, snapshot: &str, target: &str, include: Option<&str>) -> Result<()> {
        let mut a = Self::s(&["restore", snapshot, "--target", target]);
        if let Some(i) = include {
            a.push("--include".into());
            a.push(i.into());
        }
        self.run(&a).map(|_| ())
    }

    pub fn forget(&self, daily: u32, weekly: u32, monthly: u32, host: &str, prune: bool) -> Result<()> {
        if daily + weekly + monthly == 0 {
            bail!("Keine Aufbewahrungsregel gesetzt – forget würde nichts behalten");
        }
        self.run(&forget_args(daily, weekly, monthly, host, prune)).map(|_| ())
    }

    pub fn stats(&self) -> Result<RepoStats> {
        let out = self.run(&Self::s(&["stats", "--json", "--mode", "raw-data"]))?;
        parse_stats(&out).context("Unerwartete Ausgabe von restic stats")
    }

    pub fn diff(&self, a: &str, b: &str) -> Result<Vec<DiffEntry>> {
        Ok(parse_diff(&self.run(&["diff".into(), "--json".into(), a.into(), b.into()])?))
    }

    /// Prüft die Struktur und liest einen Teil der Daten (`--read-data-subset=N%`).
    pub fn check_subset(&self, percent: u32) -> Result<()> {
        self.run(&["check".into(), "--with-cache".into(), format!("--read-data-subset={percent}%")]).map(|_| ())
    }

    /// Entfernt Repository-Sperren (`restic unlock`, mit `remove_all` auch aktive).
    pub fn unlock(&self, remove_all: bool) -> Result<()> {
        self.run(&unlock_args(remove_all)).map(|_| ())
    }

    pub fn check(&self) -> Result<()> {
        self.run(&Self::s(&["check"])).map(|_| ())
    }

    /// Führt ein Backup aus. Exit-Code 3 (einige Dateien nicht lesbar) gilt als Warnung, nicht als Fehler.
    pub fn backup(&self, opts: &BackupOptions, cancel: &AtomicBool, mut on_event: impl FnMut(BackupEvent)) -> Result<BackupOutcome> {
        let args = backup_args(opts);
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
        let mut got_summary = false;
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
            // Bytes statt `lines()`: eine nicht-UTF-8-Zeile (z. B. exotischer Dateiname) würde sonst das Lesen
            // beenden, restic bliebe an der vollen Pipe hängen und `wait()` würde nie zurückkehren.
            let mut reader = BufReader::new(stdout);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
                if let Some(ev) = parse_backup_line(&String::from_utf8_lossy(&buf)) {
                    got_summary |= matches!(ev, BackupEvent::Summary { .. });
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
            Some(0) => Ok(BackupOutcome::default()),
            Some(3) => Ok(BackupOutcome { warnings: true }),
            // Snapshot wurde geschrieben (Summary erhalten), aber restic meldet Lesefehler (z. B. "Zugriff verweigert"
            // unter Windows) → Warnung statt Fehler
            Some(_) if got_summary => Ok(BackupOutcome { warnings: true }),
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
    fn find_parsed() {
        let o = r#"[{"hits":2,"snapshot":"abc","matches":[{"path":"/C/Users/x/a.txt","type":"file","size":5},{"path":"/C/Users/x/docs","type":"dir"}]},{"hits":1,"snapshot":"abc","matches":[{"path":"/b.txt","type":"file","size":1}]}]"#;
        let n = parse_find(o, 10);
        assert_eq!(n.len(), 3);
        assert_eq!((n[0].name.as_str(), n[0].size), ("a.txt", 5));
        assert_eq!(n[1].kind, "dir");
        assert_eq!(parse_find(o, 2).len(), 2);
        assert!(parse_find("kein json", 5).is_empty());
        assert_eq!(find_pattern("  foto "), "*foto*");
        assert_eq!(find_pattern("*.pdf"), "*.pdf");
    }

    #[test]
    fn diff_parsed() {
        let o = "{\"message_type\":\"change\",\"path\":\"/a\",\"modifier\":\"-\"}\n{\"message_type\":\"change\",\"path\":\"/b\",\"modifier\":\"+\"}\n{\"message_type\":\"statistics\"}\n";
        let d = parse_diff(o);
        assert_eq!(d.len(), 2);
        assert_eq!(d[1], DiffEntry { modifier: "+".into(), path: "/b".into() });
    }

    #[test]
    fn stats_parsed() {
        assert_eq!(parse_stats("{\"total_size\":2048,\"total_blob_count\":3}\n"), Some(RepoStats { size: 2048 }));
        assert_eq!(parse_stats("kaputt"), None);
    }

    #[test]
    fn forget_skips_zero() {
        assert_eq!(forget_args(7, 0, 6, "", false), ["forget", "--keep-daily", "7", "--keep-monthly", "6"]);
        assert_eq!(forget_args(1, 0, 0, "pc", true), ["forget", "--prune", "--max-unused", "10%", "--host", "pc", "--keep-daily", "1"]);
    }

    #[test]
    fn unlock_args_flag() {
        assert_eq!(unlock_args(false), ["unlock"]);
        assert_eq!(unlock_args(true), ["unlock", "--remove-all"]);
    }

    #[test]
    fn unlock_on_clean_repo() {
        if Command::new("restic").arg("version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let r = Restic { bin: "restic".into(), repo: d.path().join("repo").to_string_lossy().into(), password: "pw".into() };
        r.init().unwrap();
        r.unlock(false).unwrap();
        r.unlock(true).unwrap();
    }

    #[test]
    fn backup_args_include_excludes() {
        let src = vec!["/data".to_string()];
        let ex = vec!["*.tmp".to_string()];
        let nb = vec![".nobackup".to_string()];
        let a = backup_args(&BackupOptions {
            sources: &src,
            excludes: &ex,
            exclude_caches: true,
            exclude_if_present: &nb,
            exclude_larger_than: "500M",
            host: "pc",
            no_scan: true,
            pack_size_mib: 64,
            connections: 8,
        });
        assert_eq!(
            a,
            [
                "backup", "--json", "--read-concurrency", "4", "--host", "pc", "--exclude", "*.tmp", "--exclude-caches", "--exclude-if-present", ".nobackup",
                "--exclude-larger-than", "500M", "--no-scan", "--pack-size", "64", "-o", "local.connections=8", "/data"
            ]
        );
    }

    #[test]
    fn version_and_tuning() {
        assert_eq!(parse_version("restic 0.19.1 compiled with go1.26.4 on windows/amd64
"), Some((0, 19)));
        assert_eq!(parse_version("kaputt"), None);
        assert_eq!(tuning_for(Some((0, 19))), (64, 8));
        assert_eq!(tuning_for(Some((0, 16))), (64, 0));
        assert_eq!(tuning_for(Some((0, 13))), (0, 0));
        assert_eq!(tuning_for(None), (0, 0));
        assert_eq!(tuning_for(Some((1, 0))), (64, 8));
    }

    #[test]
    fn probe_states() {
        if Command::new("restic").arg("version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let mut r = Restic { bin: "restic".into(), repo: d.path().join("repo").to_string_lossy().into(), password: "pw".into() };
        assert_eq!(r.probe(), RepoState::NotFound);
        r.init().unwrap();
        assert_eq!(r.probe(), RepoState::Ok);
        r.password = "falsch".into();
        assert_eq!(r.probe(), RepoState::WrongPassword);
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
        let err = r.backup(&BackupOptions { sources: &[src.to_string_lossy().into()], ..Default::default() }, &AtomicBool::new(true), |_| {});
        // Entweder rechtzeitig abgebrochen oder (bei sehr kleinem Backup) schon fertig – kein Hänger, kein Panic
        if let Err(e) = err {
            assert!(e.to_string().contains("abgebrochen"));
        }
    }

    #[test]
    fn excludes_are_applied() {
        if Command::new("restic").arg("version").output().is_err() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        for dir in ["node_modules", "nb", "cache"] {
            std::fs::create_dir_all(src.join(dir)).unwrap();
        }
        std::fs::write(src.join("keep.txt"), "ok").unwrap();
        std::fs::write(src.join("skip.tmp"), "x").unwrap();
        std::fs::write(src.join("node_modules").join("x.js"), "x").unwrap();
        std::fs::write(src.join("nb").join(".nobackup"), "").unwrap();
        std::fs::write(src.join("nb").join("y.txt"), "x").unwrap();
        std::fs::write(src.join("cache").join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55").unwrap();
        std::fs::write(src.join("cache").join("z.txt"), "x").unwrap();
        std::fs::write(src.join("big.bin"), vec![1u8; 2 * 1024 * 1024]).unwrap();
        let r = Restic { bin: "restic".into(), repo: d.path().join("repo").to_string_lossy().into(), password: "pw".into() };
        r.init().unwrap();
        let (srcs, ex, nb) = (vec![src.to_string_lossy().into_owned()], vec!["*.tmp".to_string(), "node_modules".to_string()], vec![".nobackup".to_string()]);
        r.backup(
            &BackupOptions { sources: &srcs, excludes: &ex, exclude_caches: true, exclude_if_present: &nb, exclude_larger_than: "1M", host: "testhost", no_scan: true, pack_size_mib: 64, connections: 8 },
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let snaps = r.snapshots().unwrap();
        assert_eq!(snaps[0].hostname, "testhost");
        let files: Vec<String> = parse_ls(&r.run(&["ls".into(), "--json".into(), snaps[0].id.clone()]).unwrap())
            .into_iter()
            .filter(|n| n.kind == "file")
            .map(|n| n.name)
            .collect();
        assert!(files.contains(&"keep.txt".to_string()), "{files:?}");
        for gone in ["skip.tmp", "x.js", "y.txt", "z.txt", "big.bin"] {
            assert!(!files.contains(&gone.to_string()), "{gone} sollte ausgeschlossen sein: {files:?}");
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
        assert_eq!(r.probe(), RepoState::WrongPassword);
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
        assert_eq!(r.probe(), RepoState::NotFound);
        r.init().unwrap();
        assert_eq!(r.probe(), RepoState::Ok);
        let mut got_summary = false;
        r.backup(&BackupOptions { sources: &[src.to_string_lossy().into()], ..Default::default() }, &AtomicBool::new(false), |e| got_summary |= matches!(e, BackupEvent::Summary { .. })).unwrap();
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
        r.forget(1, 0, 0, "", true).unwrap();
        r.check().unwrap();
    }
}
