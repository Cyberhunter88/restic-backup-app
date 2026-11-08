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
        let base = format!(r"\\{}\{}", windows::host(cfg), cfg.share);
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

/// Maskiert ein Feld für fstab und `/proc/mounts` (Leerzeichen, Tab, Zeilenumbruch und Backslash als Oktalcode).
#[cfg(unix)]
fn mount_escape(s: &str) -> String {
    s.replace('\\', "\\134").replace(' ', "\\040").replace('\t', "\\011").replace('\n', "\\012")
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
        let want = mount_escape(&mp.to_string_lossy());
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
            "{} {} cifs user,noauto,credentials={},uid={},gid={},iocharset=utf8 0 0",
            mount_escape(&format!("//{}/{}", cfg.host, cfg.share)),
            mount_escape(&mp.to_string_lossy()),
            mount_escape(&cred_path()?.to_string_lossy()),
            md.uid(),
            md.gid()
        ))
    }

    fn in_fstab(mp: &std::path::Path) -> bool {
        let want = mount_escape(&mp.to_string_lossy());
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
    use std::sync::Mutex;

    /// Servername, unter dem die Freigabe in diesem Lauf erreichbar ist (die eingestellte Adresse
    /// oder ein Alias, siehe `mount`).
    static EFFECTIVE_HOST: Mutex<Option<String>> = Mutex::new(None);

    pub fn host(cfg: &Config) -> String {
        EFFECTIVE_HOST.lock().ok().and_then(|h| h.clone()).unwrap_or_else(|| cfg.host.clone())
    }

    fn set_host(h: &str) {
        if let Ok(mut g) = EFFECTIVE_HOST.lock() {
            *g = Some(h.to_string());
        }
    }

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

    /// Ergebnis eines Verbindungsversuchs zu einem Servernamen.
    enum Conn {
        /// Ohne Zutun zugreifbar (z. B. vom Nutzer verbunden)
        Existing,
        /// Von der App neu aufgebaut
        New,
        /// Windows hat für diesen Server schon eine Anmeldung mit anderem Benutzer (Fehler 1219)
        Conflict,
    }

    fn connect(host: &str, cfg: &Config, password: &str) -> Result<Conn> {
        let unc = format!(r"\\{host}\{}", cfg.share);
        // Echter Zugriffstest statt `net use`: ein „getrennter“ Eintrag mit falschen Anmeldedaten
        // meldete sonst Erfolg, und erst restic scheiterte mit „Benutzername oder Kennwort falsch“.
        if std::fs::read_dir(&unc).is_ok() {
            return Ok(Conn::Existing);
        }
        // Veralteten Eintrag für genau diese Freigabe entfernen (Fehler egal, falls keiner existiert)
        let _ = net(&["use", &unc, "/delete", "/y"]);
        let user = format!("/user:{}", cfg.smb_user);
        // Das Passwort geht über stdin, nicht als Argument
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
            return Ok(Conn::New);
        }
        if already_connected(&out) {
            return Ok(Conn::Conflict);
        }
        // Fallback: Passwort als Argument (kurzzeitig in der Prozessliste sichtbar)
        let out = net(&["use", &unc, password, &user, "/persistent:no"])?;
        if out.status.success() {
            return Ok(Conn::New);
        }
        if already_connected(&out) {
            return Ok(Conn::Conflict);
        }
        bail!("SMB-Verbindung fehlgeschlagen: {}", text(&out).trim());
    }

    /// Weitere Namen für denselben Server. Windows merkt sich Anmeldungen je Servername und nicht
    /// je Rechner: Unter einem anderen Namen (z. B. `nas.local` statt `192.168.178.5`) ist deshalb
    /// ein anderer Benutzer möglich, auch wenn schon Netzlaufwerke mit einem anderen Benutzer bestehen.
    fn aliases(host: &str) -> Vec<String> {
        use std::net::ToSocketAddrs;
        if host.parse::<std::net::IpAddr>().is_ok() {
            let mut c = Command::new("powershell");
            c.args(["-NoProfile", "-NonInteractive", "-Command", &format!("try {{ [System.Net.Dns]::GetHostEntry('{host}').HostName }} catch {{}}")]);
            let out = c.creation_flags(0x0800_0000).output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
            alias_names(host, &out)
        } else {
            let ips = (host, 445).to_socket_addrs().map(|a| a.filter(|s| s.is_ipv4()).map(|s| s.ip().to_string()).collect::<Vec<_>>()).unwrap_or_default();
            let mut v: Vec<String> = vec![];
            for ip in ips {
                if !v.contains(&ip) {
                    v.push(ip);
                }
            }
            v
        }
    }

    /// Aus dem per Reverse-DNS gefundenen Namen: voller und kurzer Name, ohne die Adresse selbst.
    pub(super) fn alias_names(host: &str, reverse: &str) -> Vec<String> {
        let reverse = reverse.trim().trim_end_matches('.');
        let mut v: Vec<String> = vec![];
        for n in [reverse, reverse.split('.').next().unwrap_or("")] {
            if !n.is_empty() && !n.eq_ignore_ascii_case(host) && !v.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                v.push(n.to_string());
            }
        }
        v
    }

    /// Verbindet die Freigabe. Liefert `true`, wenn die App die Verbindung neu aufgebaut hat
    /// (nur dann wird sie später wieder getrennt). Besteht schon eine Anmeldung mit anderem Benutzer
    /// am selben Server (Fehler 1219), wird die Freigabe unter einem Alias-Namen des Servers verbunden.
    pub fn mount(cfg: &Config, password: &str) -> Result<bool> {
        match connect(&cfg.host, cfg, password)? {
            Conn::Existing => return Ok(false),
            Conn::New => return Ok(true),
            Conn::Conflict => {}
        }
        for alias in aliases(&cfg.host) {
            match connect(&alias, cfg, password) {
                Ok(Conn::Existing) => {
                    set_host(&alias);
                    return Ok(false);
                }
                Ok(Conn::New) => {
                    set_host(&alias);
                    return Ok(true);
                }
                Ok(Conn::Conflict) | Err(_) => {}
            }
        }
        bail!("Windows ist mit diesem NAS bereits unter einem anderen Benutzer verbunden (z. B. über ein Netzlaufwerk), und es wurde kein zweiter Servername gefunden. In den Einstellungen statt der IP-Adresse den Namen des NAS eintragen (oder umgekehrt), damit Windows eine eigene Anmeldung erlaubt.");
    }

    pub fn unmount(cfg: &Config) -> Result<()> {
        let unc = format!(r"\\{}\{}", host(cfg), cfg.share);
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

    #[cfg(unix)]
    #[test]
    fn mount_escape_wie_fstab() {
        assert_eq!(mount_escape("/home/a b/c\\d"), "/home/a\\040b/c\\134d");
        assert_eq!(mount_escape("/ohne"), "/ohne");
    }

    #[cfg(windows)]
    #[test]
    fn alias_names_from_reverse_dns() {
        use super::windows::alias_names;
        assert_eq!(alias_names("192.168.178.128", "Ugreen-NAS.local\r\n"), ["Ugreen-NAS.local", "Ugreen-NAS"]);
        assert_eq!(alias_names("nas", "nas"), Vec::<String>::new());
        assert_eq!(alias_names("10.0.0.1", ""), Vec::<String>::new());
    }
}
