//! Wiederherstellung: Optionen und Argumente für `restic restore`, Fortschritt aus der JSON-Ausgabe und die
//! Aufteilung einer Wiederherstellung an den Originalort in einzelne restic-Aufrufe. Reine Funktionen, den
//! Prozess selbst startet `Restic::restore`.
use anyhow::{Result, bail};
use serde::Deserialize;

/// Was mit Dateien geschieht, die am Ziel schon existieren (`restic restore --overwrite`, ab 0.17).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Overwrite {
    /// Immer überschreiben (restic-Standard)
    #[default]
    Always,
    /// Nur ersetzen, wenn sich die Datei unterscheidet
    IfChanged,
    /// Vorhandene Dateien nie anfassen
    Never,
}

impl Overwrite {
    pub const ALL: [Overwrite; 3] = [Overwrite::Always, Overwrite::IfChanged, Overwrite::Never];

    /// Beschriftung für die Auswahl „Vorhandene Dateien …“
    pub fn label(self) -> &'static str {
        match self {
            Overwrite::Always => "überschreiben",
            Overwrite::IfChanged => "nur geänderte ersetzen",
            Overwrite::Never => "nicht anfassen",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Overwrite::Always => "always",
            Overwrite::IfChanged => "if-changed",
            Overwrite::Never => "never",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RestoreOptions<'a> {
    /// Snapshot-ID, bei Bedarf mit Unterordner (`<id>:/C`)
    pub snapshot: &'a str,
    pub target: &'a str,
    /// Nur diesen Pfad wiederherstellen (bei `<id>:<Unterordner>` relativ zum Unterordner)
    pub include: Option<&'a str>,
    pub overwrite: Overwrite,
    /// Wiederhergestellte Dateien anschließend prüfen (`--verify`)
    pub verify: bool,
    /// Ziel ist der Originalort: nur mit restic ab 0.17 (Unterordner-Syntax und `--overwrite`)
    pub in_place: bool,
}

/// `--json` (Fortschritt) und `--overwrite` gibt es ab restic 0.17; ältere Versionen überschreiben immer.
pub fn restore_args(o: &RestoreOptions, version: Option<(u32, u32)>) -> Vec<String> {
    let mut a = vec!["restore".to_string(), o.snapshot.into(), "--target".into(), o.target.into()];
    if version.is_some_and(|v| v >= (0, 17)) {
        a.push("--json".into());
        if o.overwrite != Overwrite::Always {
            a.extend(["--overwrite".into(), o.overwrite.flag().into()]);
        }
    }
    if o.verify {
        a.push("--verify".into());
    }
    if let Some(i) = o.include {
        a.extend(["--include".into(), i.into()]);
    }
    a
}

/// Fortschritt (0.0 bis 1.0) aus einer Zeile von `restic restore --json`.
pub fn parse_restore_progress(line: &str) -> Option<f64> {
    #[derive(Deserialize)]
    struct Raw {
        message_type: String,
        #[serde(default)]
        percent_done: f64,
    }
    let r: Raw = serde_json::from_str(line).ok()?;
    (r.message_type == "status").then_some(r.percent_done)
}

/// Ein einzelner restic-Aufruf einer Wiederherstellung.
#[derive(Debug, Clone, PartialEq)]
pub struct RestoreJob {
    pub snapshot: String,
    pub target: String,
    pub include: Option<String>,
}

/// Aufrufe, die einen Pfad (`include`, sonst den ganzen Snapshot) an den Originalort zurückschreiben.
/// Linux: Ziel `/`. Windows: restic legt den Laufwerksbuchstaben als obersten Ordner ab (`/C/Users/…`), ein Restore
/// nach `C:\` ergäbe `C:\C\Users\…`. Deshalb je Laufwerk ein Aufruf mit Unterordner-Syntax: `<id>:/C` nach `C:\`.
pub fn in_place_jobs(id: &str, include: Option<&str>, snapshot_paths: &[String], windows: bool) -> Result<Vec<RestoreJob>> {
    if !windows {
        return Ok(vec![RestoreJob { snapshot: id.into(), target: "/".into(), include: include.map(String::from) }]);
    }
    let job = |drive: char, include: Option<String>| RestoreJob { snapshot: format!("{id}:/{drive}"), target: format!("{drive}:\\"), include };
    match include {
        Some(p) => {
            let rest = p.trim_start_matches('/');
            let (first, tail) = rest.split_once('/').unwrap_or((rest, ""));
            let mut cs = first.chars();
            match (cs.next(), cs.next()) {
                (Some(d), None) if d.is_ascii_alphabetic() => Ok(vec![job(d, (!tail.is_empty()).then(|| format!("/{tail}")))]),
                _ => bail!("Der Originalort von „{p}“ lässt sich nicht bestimmen (kein Laufwerksbuchstabe)"),
            }
        }
        None => {
            let mut drives: Vec<char> = vec![];
            for p in snapshot_paths {
                let mut cs = p.chars();
                match (cs.next(), cs.next()) {
                    (Some(d), Some(':')) if d.is_ascii_alphabetic() => {
                        if !drives.contains(&d) {
                            drives.push(d);
                        }
                    }
                    _ => bail!("Der Originalort von „{p}“ lässt sich nicht bestimmen (kein Laufwerksbuchstabe)"),
                }
            }
            if drives.is_empty() {
                bail!("Der Snapshot enthält keine Pfade");
            }
            Ok(drives.into_iter().map(|d| job(d, None)).collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_args_by_version() {
        let o = RestoreOptions { snapshot: "abc", target: "/ziel", include: Some("/a b"), overwrite: Overwrite::IfChanged, verify: true, in_place: false };
        assert_eq!(restore_args(&o, Some((0, 18))), ["restore", "abc", "--target", "/ziel", "--json", "--overwrite", "if-changed", "--verify", "--include", "/a b"]);
        // Ältere Versionen kennen weder `--json` (Fortschritt) noch `--overwrite`
        assert_eq!(restore_args(&o, Some((0, 16))), ["restore", "abc", "--target", "/ziel", "--verify", "--include", "/a b"]);
        assert_eq!(restore_args(&o, None), ["restore", "abc", "--target", "/ziel", "--verify", "--include", "/a b"]);
        // Standard „überschreiben“ braucht keinen Schalter
        let plain = RestoreOptions { snapshot: "abc", target: "/ziel", ..Default::default() };
        assert_eq!(restore_args(&plain, Some((0, 18))), ["restore", "abc", "--target", "/ziel", "--json"]);
    }

    #[test]
    fn restore_progress_parsed() {
        assert_eq!(parse_restore_progress("{\"message_type\":\"status\",\"percent_done\":0.25,\"files_restored\":3}\n"), Some(0.25));
        assert_eq!(parse_restore_progress("{\"message_type\":\"summary\",\"files_restored\":3}"), None);
        assert_eq!(parse_restore_progress("kein json"), None);
    }

    #[test]
    fn in_place_jobs_linux_and_windows() {
        let job = |s: &str, t: &str, i: Option<&str>| RestoreJob { snapshot: s.into(), target: t.into(), include: i.map(String::from) };
        // Linux: ein Aufruf nach `/`
        assert_eq!(in_place_jobs("abc", Some("/home/x/a.txt"), &[], false).unwrap(), vec![job("abc", "/", Some("/home/x/a.txt"))]);
        assert_eq!(in_place_jobs("abc", None, &["/home/x".into()], false).unwrap(), vec![job("abc", "/", None)]);
        // Windows: Laufwerk als Unterordner des Snapshots, der Rest relativ dazu
        assert_eq!(in_place_jobs("abc", Some("/C/Users/x/a.txt"), &[], true).unwrap(), vec![job("abc:/C", "C:\\", Some("/Users/x/a.txt"))]);
        assert_eq!(in_place_jobs("abc", Some("/D"), &[], true).unwrap(), vec![job("abc:/D", "D:\\", None)]);
        // Ganzer Snapshot: je Laufwerk ein Aufruf
        let paths = ["C:\\Users\\x".to_string(), "D:\\Daten".into(), "C:\\Projekte".into()];
        assert_eq!(in_place_jobs("abc", None, &paths, true).unwrap(), vec![job("abc:/C", "C:\\", None), job("abc:/D", "D:\\", None)]);
        // Ohne Laufwerksbuchstaben (z. B. Netzwerkpfad) lieber ablehnen als falsch schreiben
        assert!(in_place_jobs("abc", Some("/server/freigabe/a.txt"), &[], true).is_err());
        assert!(in_place_jobs("abc", None, &["\\\\server\\freigabe".to_string()], true).is_err());
        assert!(in_place_jobs("abc", None, &[], true).is_err());
    }
}
