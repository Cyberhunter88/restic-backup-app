//! Update-Prüfung gegen die neueste GitHub-Release-Version (per `curl`, keine zusätzliche Abhängigkeit).
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::process::Command;

const REPO: &str = "Cyberhunter88/restic-backup-app";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

pub struct Latest {
    pub version: String,
    pub url: String,
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.trim().trim_start_matches('v').split('.');
    let p = (it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.split(['-', '+']).next()?.parse().ok()?);
    Some(p)
}

/// `true`, wenn `latest` neuer als `current` ist.
pub fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

pub fn parse_release(json: &str) -> Result<Latest> {
    let r: Release = serde_json::from_str(json).context("Unerwartete Antwort von GitHub")?;
    Ok(Latest { version: r.tag_name.trim_start_matches('v').to_string(), url: r.html_url })
}

pub fn fetch_latest() -> Result<Latest> {
    let mut c = Command::new("curl");
    c.args(["-fsSL", "--max-time", "15", "-H", "Accept: application/vnd.github+json", "-H", "User-Agent: restic-backup-app"])
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.output().context("curl nicht verfügbar")?;
    if !out.status.success() {
        bail!("Update-Server nicht erreichbar oder noch kein Release vorhanden");
    }
    parse_release(&String::from_utf8_lossy(&out.stdout))
}

/// Öffnet eine URL im Standardbrowser.
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).creation_flags(0x0800_0000).spawn();
    }
    #[cfg(unix)]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare() {
        assert!(is_newer("v0.3.0", "0.2.0"));
        assert!(is_newer("0.2.1", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
        assert!(!is_newer("kaputt", "0.2.0"));
    }

    #[test]
    fn parses_release() {
        let l = parse_release(r#"{"tag_name":"v0.2.0","html_url":"https://x/y","other":1}"#).unwrap();
        assert_eq!(l.version, "0.2.0");
        assert_eq!(l.url, "https://x/y");
    }
}
