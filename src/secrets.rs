use anyhow::{Context, Result};
use keyring_core::Entry;
use std::sync::Mutex;

const SERVICE: &str = "restic-backup-app";

#[derive(Clone, Copy)]
pub enum Secret {
    Smb,
    Repo,
}

impl Secret {
    fn key(self) -> &'static str {
        match self {
            Secret::Smb => "smb-password",
            Secret::Repo => "restic-repo-password",
        }
    }
}

/// Ist der Speicher der Plattform schon verbunden?
static STORE_READY: Mutex<bool> = Mutex::new(false);

/// Verbindet beim ersten Zugriff mit dem System-Schlüsselbund (Linux: Secret Service, z. B. KWallet oder GNOME
/// Keyring; Windows: Anmeldeinformationsverwaltung). Scheitert das (kurz nach dem Anmelden läuft der Dienst oft
/// noch nicht), versucht es der nächste Aufruf erneut, statt den Fehler für den ganzen Prozess zu behalten.
fn ensure_store() -> Result<()> {
    let mut ready = STORE_READY.lock().unwrap_or_else(|e| e.into_inner());
    if !*ready {
        #[cfg(windows)]
        let store = windows_native_keyring_store::Store::new();
        #[cfg(not(windows))]
        let store = zbus_secret_service_keyring_store::Store::new();
        keyring_core::set_default_store(store.context("System-Schlüsselbund nicht erreichbar")?);
        *ready = true;
    }
    Ok(())
}

/// Ist der System-Schlüsselbund nutzbar? Versucht bei Bedarf die Verbindung (erneut).
pub fn store_ready() -> bool {
    ensure_store().is_ok()
}

pub fn get(s: Secret) -> Result<Option<String>> {
    ensure_store()?;
    match Entry::new(SERVICE, s.key())?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set(s: Secret, value: &str) -> Result<()> {
    ensure_store()?;
    Entry::new(SERVICE, s.key())?.set_password(value)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Braucht einen laufenden Schlüsselbund: `cargo test live_store -- --ignored`
    #[test]
    #[ignore]
    fn live_store() {
        assert!(store_ready());
        // Lesen darf nicht scheitern (ein fehlender Eintrag ist `None`, kein Fehler)
        println!("Repo-Passwort gespeichert: {}", get(Secret::Repo).unwrap().is_some());
        println!("SMB-Passwort gespeichert: {}", get(Secret::Smb).unwrap().is_some());
    }
}
