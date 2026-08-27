use std::fs;
use std::path::PathBuf;

use directories::ProjectDirs;
use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};

const SERVICE: &str = "img2omezarr";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub s3_profiles: Vec<S3Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Profile {
    pub name: String,
    pub bucket: String,
    pub prefix: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
    pub access_key_id_ref: String,
    pub secret_access_key_ref: String,
}

#[derive(Debug, Clone)]
pub struct ProfileSecrets {
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
}

impl AppConfig {
    pub fn profile_names(&self) -> Vec<String> {
        self.s3_profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect()
    }

    pub fn find_profile(&self, name: &str) -> Option<&S3Profile> {
        self.s3_profiles.iter().find(|profile| profile.name == name)
    }

    pub fn upsert_profile(&mut self, profile: S3Profile) {
        if let Some(existing) = self
            .s3_profiles
            .iter_mut()
            .find(|existing| existing.name == profile.name)
        {
            *existing = profile;
        } else {
            self.s3_profiles.push(profile);
            self.s3_profiles
                .sort_by(|left, right| left.name.cmp(&right.name));
        }
    }

    pub fn remove_profile(&mut self, name: &str) -> Option<S3Profile> {
        let index = self
            .s3_profiles
            .iter()
            .position(|profile| profile.name == name)?;
        Some(self.s3_profiles.remove(index))
    }
}

pub fn load() -> anyhow::Result<AppConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

pub fn save(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(config)?)?;
    Ok(())
}

pub fn profile_for_fields(
    name: String,
    bucket: String,
    prefix: String,
    region: Option<String>,
    endpoint: Option<String>,
) -> S3Profile {
    S3Profile {
        access_key_id_ref: access_key_id_ref(&name),
        secret_access_key_ref: secret_access_key_ref(&name),
        name,
        bucket,
        prefix,
        region,
        endpoint,
    }
}

pub fn save_profile_secrets(
    profile: &S3Profile,
    access_key_id: Option<&str>,
    secret_access_key: Option<&str>,
) -> anyhow::Result<()> {
    save_or_delete_secret(&profile.access_key_id_ref, access_key_id)?;
    save_or_delete_secret(&profile.secret_access_key_ref, secret_access_key)?;
    Ok(())
}

pub fn load_profile_secrets(profile: &S3Profile) -> anyhow::Result<ProfileSecrets> {
    Ok(ProfileSecrets {
        access_key_id: load_secret(&profile.access_key_id_ref)?,
        secret_access_key: load_secret(&profile.secret_access_key_ref)?,
    })
}

pub fn delete_profile_secrets(profile: &S3Profile) -> anyhow::Result<()> {
    delete_secret(&profile.access_key_id_ref)?;
    delete_secret(&profile.secret_access_key_ref)?;
    Ok(())
}

fn config_path() -> anyhow::Result<PathBuf> {
    let dirs = ProjectDirs::from("org", "henriksson-lab", "img2omezarr")
        .ok_or_else(|| anyhow::anyhow!("could not determine user config directory"))?;
    Ok(dirs.config_dir().join("config.toml"))
}

fn access_key_id_ref(profile_name: &str) -> String {
    format!("s3:{profile_name}:access_key_id")
}

fn secret_access_key_ref(profile_name: &str) -> String {
    format!("s3:{profile_name}:secret_access_key")
}

fn save_or_delete_secret(reference: &str, value: Option<&str>) -> anyhow::Result<()> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    if let Some(value) = value {
        entry(reference)?.set_password(value)?;
    } else {
        delete_secret(reference)?;
    }
    Ok(())
}

fn load_secret(reference: &str) -> anyhow::Result<Option<String>> {
    match entry(reference)?.get_password() {
        Ok(value) => Ok(Some(value)),
        Err(KeyringError::NoEntry) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

fn delete_secret(reference: &str) -> anyhow::Result<()> {
    match entry(reference)?.delete_credential() {
        Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

fn entry(reference: &str) -> anyhow::Result<Entry> {
    Ok(Entry::new(SERVICE, reference)?)
}
