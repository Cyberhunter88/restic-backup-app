//! Hilfen rund um Updates: Das Repo ist privat, daher gibt es keine anonyme API-Abfrage;
//! die App öffnet nur die Releases-Seite im Browser.
use std::process::Command;

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
