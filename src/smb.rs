use crate::config::Config;
use anyhow::Result;
#[cfg(unix)]
use std::path::PathBuf;

/// Pfad des restic-Repos auf der Freigabe (nach erfolgreichem Mount).
pub fn repo_path(cfg: &Config) -> Result<String> {
    let sub = cfg.subpath.trim_matches(['/', '\\']);
    #[cfg(unix)]
    {
        let mut p: PathBuf = linux::mount_point()?;
        if !sub.is_empty() {
            p.push(sub);
        }
        Ok(p.to_string_lossy().into_owned())
    }
    #[cfg(windows)]
    {
        let base = format!(r"\\{}\{}", cfg.host, cfg.share);
        Ok(if sub.is_empty() { base } else { format!(r"{}\{}", base, sub.replace('/', r"\")) })
    }
}

/// Antwortet das NAS auf dem SMB-Port (445)? Kurzer Test, bevor ein Zeitplan-Lauf Zeit verbraucht.
pub fn host_reachable(host: &str) -> bool {
    use std::net::ToSocketAddrs;
    let Ok(addrs) = (host.trim(), 445).to_socket_addrs() else { return false };
    addrs.into_iter().any(|a| std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_secs(3)).is_ok())
}

/// Freier Platz (Bytes) auf dem Datenträger des Repo-Pfads; unter Windows (UNC) nicht ermittelbar.
pub fn free_space(path: &str) -> Option<u64> {
    #[cfg(unix)]
    {
        let out = std::process::Command::new("df").args(["-Pk", path]).output().ok()?;
        parse_df(&String::from_utf8_lossy(&out.stdout))
    }
    #[cfg(windows)]
    {
        let _ = path;
        None
    }
}

/// Liest „verfügbar“ (1K-Blöcke) aus der `df -Pk`-Ausgabe.
#[cfg_attr(windows, allow(dead_code))]
pub fn parse_df(output: &str) -> Option<u64> {
    let line = output.lines().nth(1)?;
    line.split_whitespace().nth(3)?.parse::<u64>().ok().map(|k| k * 1024)
}

#[cfg(unix)]
pub use linux::{fstab_line, mount, unmount, write_credentials};
#[cfg(windows)]
pub use windows::{mount, unmount};

#[cfg(unix)]
mod linux {
    use super::*;
    use anyhow::{Context, bail};
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::process::Command;

    pub fn mount_point() -> Result<PathBuf> {
        let p = crate::config::data_dir()?.join("mnt");
        std::fs::create_dir_all(&p)?;
        Ok(p)
    }

    fn is_mounted(mp: &std::path::Path) -> bool {
        let want = mp.to_string_lossy().replace(' ', "\\040");
        std::fs::read_to_string("/proc/mounts")
            .map(|s| s.lines().any(|l| l.split(' ').nth(1) == Some(want.as_str())))
            .unwrap_or(false)
    }

    fn cred_path() -> Result<PathBuf> {
        Ok(crate::config::data_dir()?.join("smb.cred"))
    }

    /// Schreibt die Credentials-Datei (0600) neu.
    pub fn write_credentials(cfg: &Config, password: &str) -> Result<()> {
        let cred = cred_path()?;
        std::fs::create_dir_all(cred.parent().unwrap())?;
        let _ = std::fs::remove_file(&cred);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&cred)?;
        writeln!(f, "username={}\npassword={}", cfg.smb_user, password)?;
        Ok(())
    }

    /// fstab-Zeile für Mounts ohne Root-Rückfrage (`user,noauto`).
    pub fn fstab_line(cfg: &Config) -> Result<String> {
        let mp = mount_point()?;
        let md = std::fs::metadata(&mp)?;
        Ok(format!(
            "//{}/{} {} cifs user,noauto,credentials={},uid={},gid={},iocharset=utf8 0 0",
            cfg.host, cfg.share, mp.display(), cred_path()?.display(), md.uid(), md.gid()
        ))
    }

    fn in_fstab(mp: &std::path::Path) -> bool {
        let want = mp.to_string_lossy().replace(' ', "\\040");
        std::fs::read_to_string("/etc/fstab")
            .map(|s| s.lines().any(|l| !l.trim_start().starts_with('#') && l.split_whitespace().nth(1) == Some(want.as_str())))
            .unwrap_or(false)
    }

    /// Bindet die Freigabe per `mount.cifs` (über pkexec für Root-Rechte) ein.
    /// Liefert `true`, wenn die App die Freigabe neu eingebunden hat.
    pub fn mount(cfg: &Config, password: &str) -> Result<bool> {
        let mp = mount_point()?;
        if is_mounted(&mp) {
            return Ok(false);
        }
        // fstab-Eintrag mit `user`: Mount ohne Root und ohne Dialog (auch für Zeitplan)
        if in_fstab(&mp) {
            write_credentials(cfg, password)?;
            let out = Command::new("mount").arg(&mp).output().context("mount konnte nicht gestartet werden")?;
            if !out.status.success() {
                bail!("SMB-Mount fehlgeschlagen: {}", String::from_utf8_lossy(&out.stderr).trim());
            }
            return Ok(true);
        }
        let uid = std::fs::metadata(&mp)?.uid();
        let gid = std::fs::metadata(&mp)?.gid();
        // Credentials-Datei (0600), damit das Passwort nicht in der Prozessliste auftaucht
        write_credentials(cfg, password)?;
        let cred = cred_path()?;
        let out = Command::new("pkexec")
            .arg("mount")
            .args(["-t", "cifs", &format!("//{}/{}", cfg.host, cfg.share)])
            .arg(&mp)
            .args(["-o", &format!("credentials={},uid={uid},gid={gid},iocharset=utf8", cred.display())])
            .output()
            .context("pkexec/mount konnte nicht gestartet werden");
        let _ = std::fs::remove_file(&cred);
        let out = out?;
        if !out.status.success() {
            bail!("SMB-Mount fehlgeschlagen: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(true)
    }

    pub fn unmount(_cfg: &Config) -> Result<()> {
        let mp = mount_point()?;
        if !is_mounted(&mp) {
            return Ok(());
        }
        let out = if in_fstab(&mp) {
            Command::new("umount").arg(&mp).output()?
        } else {
            Command::new("pkexec").arg("umount").arg(&mp).output()?
        };
        if !out.status.success() {
            bail!("umount fehlgeschlagen: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use anyhow::bail;
    use std::io::Write;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    fn net(args: &[&str]) -> Result<std::process::Output> {
        Ok(Command::new("net").args(args).creation_flags(0x0800_0000).output()?)
    }

    fn text(o: &std::process::Output) -> String {
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }

    /// Fehler 1219 = bereits mit anderen Anmeldedaten verbunden.
    fn already_connected(o: &std::process::Output) -> bool {
        text(o).contains("1219")
    }

    /// Verbindet die Freigabe. Das Passwort wird über stdin übergeben (nicht als Argument).
    /// Liefert `true`, wenn die App die Verbindung neu aufgebaut hat.
    pub fn mount(cfg: &Config, password: &str) -> Result<bool> {
        let unc = format!(r"\\{}\{}", cfg.host, cfg.share);
        // Bereits verbunden (z. B. durch den Nutzer)? Dann nichts anfassen und später nicht trennen.
        if net(&["use", &unc]).map(|o| o.status.success()).unwrap_or(false) {
            return Ok(false);
        }
        let user = format!("/user:{}", cfg.smb_user);
        let mut child = Command::new("net")
            .args(["use", &unc, "*", &user, "/persistent:no"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(0x0800_0000)
            .spawn()?;
        if let Some(mut si) = child.stdin.take() {
            let _ = writeln!(si, "{password}");
        }
        let out = child.wait_with_output()?;
        if out.status.success() {
            return Ok(true);
        }
        if already_connected(&out) {
            return Ok(false);
        }
        // Fallback: Passwort als Argument (kurzzeitig in der Prozessliste sichtbar)
        let out = net(&["use", &unc, password, &user, "/persistent:no"])?;
        if out.status.success() {
            return Ok(true);
        }
        if already_connected(&out) {
            return Ok(false);
        }
        bail!("SMB-Verbindung fehlgeschlagen: {}", text(&out).trim());
    }

    pub fn unmount(cfg: &Config) -> Result<()> {
        let unc = format!(r"\\{}\{}", cfg.host, cfg.share);
        net(&["use", &unc, "/delete", "/y"]).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn df_parsed() {
        let o = "Filesystem 1024-blocks Used Available Capacity Mounted on\n//nas/share 1000 400 600 40% /mnt\n";
        assert_eq!(parse_df(o), Some(600 * 1024));
        assert_eq!(parse_df("leer"), None);
    }
}
