use anyhow::Result;
use keyring::Entry;

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

pub fn get(s: Secret) -> Result<Option<String>> {
    match Entry::new(SERVICE, s.key())?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn set(s: Secret, value: &str) -> Result<()> {
    Entry::new(SERVICE, s.key())?.set_password(value)?;
    Ok(())
}
