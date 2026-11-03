use anyhow::{Context, Result, bail};
use std::process::Command;

fn parse_time(t: &str) -> Result<(u32, u32)> {
    let (h, m) = t.split_once(':').context("Zeit muss HH:MM sein")?;
    let (h, m): (u32, u32) = (h.parse()?, m.parse()?);
    if h > 23 || m > 59 {
        bail!("Ungültige Uhrzeit");
    }
    Ok((h, m))
}

#[cfg(unix)]
pub fn install(time: &str) -> Result<()> {
    let (h, m) = parse_time(time)?;
    let exe = std::env::current_exe()?;
    let dir = directories::BaseDirs::new().context("Kein Home")?.config_dir().join("systemd/user");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("restic-backup-app.service"),
        format!("[Unit]\nDescription=restic Backup auf NAS\n\n[Service]\nType=oneshot\nExecStart=\"{}\" --run-backup\n", exe.display()),
    )?;
    std::fs::write(
        dir.join("restic-backup-app.timer"),
        format!("[Unit]\nDescription=Täglicher restic Backup\n\n[Timer]\nOnCalendar=*-*-* {h:02}:{m:02}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"),
    )?;
    run("systemctl", &["--user", "daemon-reload"])?;
    run("systemctl", &["--user", "enable", "--now", "restic-backup-app.timer"])
}

#[cfg(unix)]
pub fn remove() -> Result<()> {
    run("systemctl", &["--user", "disable", "--now", "restic-backup-app.timer"])
}

#[cfg(windows)]
pub fn install(time: &str) -> Result<()> {
    parse_time(time)?;
    let exe = std::env::current_exe()?;
    run("schtasks", &["/create", "/f", "/sc", "daily", "/st", time, "/tn", "restic-backup-app", "/tr", &format!("\"{}\" --run-backup", exe.display())])
}

#[cfg(windows)]
pub fn remove() -> Result<()> {
    run("schtasks", &["/delete", "/f", "/tn", "restic-backup-app"])
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd).args(args).output().with_context(|| format!("{cmd} nicht startbar"))?;
    if !out.status.success() {
        bail!("{cmd} fehlgeschlagen: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_time;
    #[test]
    fn time() {
        assert_eq!(parse_time("02:30").unwrap(), (2, 30));
        assert!(parse_time("25:00").is_err());
        assert!(parse_time("abc").is_err());
    }
}
