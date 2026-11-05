//! Programm beim Anmelden automatisch (minimiert) starten.
use anyhow::{Context, Result};

#[cfg(unix)]
fn desktop_entry(exe: &std::path::Path) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=restic Backup\nComment=restic-Backup auf NAS\nExec=\"{}\" --autostart\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    )
}

#[cfg(unix)]
fn entry_path() -> Result<std::path::PathBuf> {
    Ok(directories::BaseDirs::new().context("Kein Home")?.config_dir().join("autostart/restic-backup-app.desktop"))
}

#[cfg(unix)]
pub fn is_enabled() -> bool {
    entry_path().map(|p| p.exists()).unwrap_or(false)
}

#[cfg(unix)]
pub fn set(enabled: bool) -> Result<()> {
    let p = entry_path()?;
    if enabled {
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(&p, desktop_entry(&crate::schedule::exe_path()?))?;
    } else if p.exists() {
        std::fs::remove_file(&p)?;
    }
    Ok(())
}

#[cfg(windows)]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
fn reg(args: &[&str]) -> Result<std::process::Output> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg").args(args).creation_flags(0x0800_0000).output().context("reg nicht startbar")
}

#[cfg(windows)]
pub fn is_enabled() -> bool {
    reg(&["query", RUN_KEY, "/v", "restic-backup-app"]).map(|o| o.status.success()).unwrap_or(false)
}

#[cfg(windows)]
pub fn set(enabled: bool) -> Result<()> {
    let out = if enabled {
        let cmd = format!("\"{}\" --autostart", crate::schedule::exe_path()?.display());
        reg(&["add", RUN_KEY, "/v", "restic-backup-app", "/t", "REG_SZ", "/d", &cmd, "/f"])?
    } else {
        reg(&["delete", RUN_KEY, "/v", "restic-backup-app", "/f"])?
    };
    if !out.status.success() && enabled {
        anyhow::bail!("Autostart konnte nicht eingerichtet werden: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    #[test]
    fn desktop_entry_quotes_path_and_flag() {
        let s = super::desktop_entry(std::path::Path::new("/opt/my apps/restic-backup-app"));
        assert!(s.contains("Exec=\"/opt/my apps/restic-backup-app\" --autostart"));
        assert!(s.starts_with("[Desktop Entry]"));
    }
}
