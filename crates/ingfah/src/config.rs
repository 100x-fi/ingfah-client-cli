use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use ingfah_client::ApiKey;
use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "ingfah-cli";
/// Set only by the test suite, so tests never read or write the developer's keychain.
const DISABLE_KEYRING_ENV: &str = "INGFAH_TEST_DISABLE_KEYRING";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// `$XDG_CONFIG_HOME/ingfah/config.toml`, else `~/.config/ingfah/config.toml`.
pub fn config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| directories::BaseDirs::new().map(|d| d.home_dir().join(".config")))?;
    Some(base.join("ingfah").join("config.toml"))
}

impl ConfigFile {
    pub fn load() -> Result<Self> {
        let Some(path) = config_path() else {
            return Ok(Self::default());
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = config_path().context("cannot locate a home directory for the config file")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(path)
    }

    pub fn base_url(&self, profile: &str) -> Option<&str> {
        self.profiles.get(profile)?.base_url.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySource {
    Flag,
    Env,
    Keyring,
}

impl KeySource {
    pub fn describe(self, profile: &str) -> String {
        match self {
            Self::Flag => "--api-key flag".to_owned(),
            Self::Env => "INGFAH_API_KEY environment variable".to_owned(),
            Self::Keyring => {
                format!("system keyring (service {KEYRING_SERVICE}, profile {profile})")
            }
        }
    }
}

/// `--api-key`, then `INGFAH_API_KEY`, then the keyring. The keyring is only consulted
/// when neither is set.
pub fn resolve_key(
    flag: Option<&str>,
    env: Option<&str>,
    keyring: impl FnOnce() -> Result<Option<String>>,
) -> Result<Option<(ApiKey, KeySource)>> {
    let non_empty = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(ApiKey::new);
    if let Some(key) = non_empty(flag) {
        return Ok(Some((key, KeySource::Flag)));
    }
    if let Some(key) = non_empty(env) {
        return Ok(Some((key, KeySource::Env)));
    }
    Ok(keyring()?
        .as_deref()
        .and_then(|k| non_empty(Some(k)))
        .map(|k| (k, KeySource::Keyring)))
}

fn keyring_disabled() -> bool {
    std::env::var_os(DISABLE_KEYRING_ENV).is_some()
}

fn entry(profile: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, profile).context("opening the system keyring")
}

pub fn keyring_get(profile: &str) -> Result<Option<String>> {
    if keyring_disabled() {
        return Ok(None);
    }
    match entry(profile)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("reading the key from the system keyring"),
    }
}

pub fn keyring_set(profile: &str, key: &ApiKey) -> Result<()> {
    anyhow::ensure!(!keyring_disabled(), "the keyring is disabled");
    entry(profile)?
        .set_password(key.expose())
        .context("storing the key in the system keyring")
}

/// Returns false when there was no stored key.
pub fn keyring_delete(profile: &str) -> Result<bool> {
    anyhow::ensure!(!keyring_disabled(), "the keyring is disabled");
    match entry(profile)?.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(e).context("deleting the key from the system keyring"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn untouched() -> Result<Option<String>> {
        panic!("keyring must not be read when a flag or env key is set")
    }

    #[test]
    fn flag_wins_over_env() {
        let resolved = resolve_key(Some("from-flag"), Some("from-env"), untouched)
            .ok()
            .flatten();
        assert_eq!(resolved, Some((ApiKey::new("from-flag"), KeySource::Flag)));
    }

    #[test]
    fn env_used_when_no_flag() {
        let resolved = resolve_key(None, Some("from-env"), untouched)
            .ok()
            .flatten();
        assert_eq!(resolved, Some((ApiKey::new("from-env"), KeySource::Env)));
    }

    #[test]
    fn empty_env_falls_through_to_keyring() {
        let resolved = resolve_key(None, Some("  "), || Ok(Some("stored".to_owned())))
            .ok()
            .flatten();
        assert_eq!(resolved, Some((ApiKey::new("stored"), KeySource::Keyring)));
    }

    #[test]
    fn nothing_anywhere_resolves_to_none() {
        let resolved = resolve_key(None, None, || Ok(None)).ok().flatten();
        assert_eq!(resolved, None);
    }
}
