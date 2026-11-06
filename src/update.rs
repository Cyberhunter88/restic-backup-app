//! Hilfen rund um Updates: Suche nach einer neueren Version über die öffentliche GitHub-API
//! (nur Lesen, es werden keine Daten gesendet) und Öffnen der Releases-Seite im Browser.
use anyhow::{Context, Result};
use std::process::Command;

const LATEST_API: &str = "https://api.github.com/repos/Cyberhunter88/restic-backup-app/releases/latest";

/// `v0.3.8` / `0.3.8` → `[0, 3, 8]`; Vorabversionen und Unbekanntes → `None`.
fn parse_version(s: &str) -> Option<Vec<u64>> {
    s.trim().trim_start_matches('v').split('.').map(|p| p.parse().ok()).collect()
}

/// Ist `tag` eine neuere Version als `current`?
pub fn newer(current: &str, tag: &str) -> bool {
    matches!((parse_version(current), parse_version(tag)), (Some(c), Some(t)) if t > c)
}

/// Liefert die neueste veröffentlichte Version (ohne `v`), falls sie neuer als die laufende ist.
pub fn check_latest() -> Result<Option<String>> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(std::time::Duration::from_secs(10))).build().into();
    let mut resp = agent.get(LATEST_API).header("User-Agent", "restic-backup-app").header("Accept", "application/vnd.github+json").call()?;
    let body = resp.body_mut().read_to_string()?;
    let tag = tag_from_json(&body)?;
    Ok(newer(env!("CARGO_PKG_VERSION"), &tag).then(|| tag.trim_start_matches('v').to_string()))
}

fn tag_from_json(body: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(body).context("Antwort von GitHub nicht lesbar")?;
    Ok(v["tag_name"].as_str().context("Keine Version in der Antwort")?.to_string())
}

pub const RELEASES_URL: &str = "https://github.com/Cyberhunter88/restic-backup-app/releases/latest";

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
        assert!(newer("0.3.7", "v0.3.8"));
        assert!(newer("0.3.7", "0.4.0"));
        assert!(newer("0.3.9", "v0.3.10"));
        assert!(!newer("0.3.7", "v0.3.7"));
        assert!(!newer("0.3.7", "v0.3.6"));
        assert!(!newer("0.3.7", "v0.4.0-rc1"));
        assert!(!newer("0.3.7", "kaputt"));
    }

    /// Echte Abfrage von GitHub (Netzwerk): `cargo test live_check -- --ignored`
    #[test]
    #[ignore]
    fn live_check() {
        let r = check_latest().unwrap();
        println!("neuere Version: {r:?}");
    }

    #[test]
    fn tag_parsed() {
        assert_eq!(tag_from_json("{\"tag_name\": \"v0.3.8\", \"name\": \"x\"}").unwrap(), "v0.3.8");
        assert!(tag_from_json("{}").is_err());
        assert!(tag_from_json("kein json").is_err());
    }
}
