use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const CONFIG_DIR_ENV: &str = "IMG2OMEZARR_GUI_CONFIG_DIR";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub default_profile: Option<String>,
    #[serde(default)]
    pub settings: GuiSettings,
    #[serde(default)]
    pub s3_profiles: Vec<S3Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuiSettings {
    #[serde(default = "default_ngff_version")]
    pub ngff_version: String,
    #[serde(default = "default_tile_width")]
    pub tile_width: i32,
    #[serde(default = "default_tile_height")]
    pub tile_height: i32,
    #[serde(default = "default_chunk_depth")]
    pub chunk_depth: i32,
    #[serde(default = "default_target_min_size")]
    pub target_min_size: i32,
    #[serde(default)]
    pub use_existing_resolutions: bool,
    #[serde(default = "default_downsampling")]
    pub downsampling: String,
    #[serde(default = "default_compression")]
    pub compression: String,
    #[serde(default = "default_compression_level")]
    pub compression_level: i32,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default = "default_write_omero")]
    pub write_omero: bool,
    #[serde(default)]
    pub staged_upload: bool,
    #[serde(default = "default_upload_mode")]
    pub upload_mode: String,
    #[serde(default)]
    pub last_output_dir: Option<String>,
}

impl Default for GuiSettings {
    fn default() -> Self {
        Self {
            ngff_version: default_ngff_version(),
            tile_width: default_tile_width(),
            tile_height: default_tile_height(),
            chunk_depth: default_chunk_depth(),
            target_min_size: default_target_min_size(),
            use_existing_resolutions: false,
            downsampling: default_downsampling(),
            compression: default_compression(),
            compression_level: default_compression_level(),
            overwrite: false,
            write_omero: default_write_omero(),
            staged_upload: false,
            upload_mode: default_upload_mode(),
            last_output_dir: None,
        }
    }
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CredentialsFile {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
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
    load_from_path(&path)
}

pub fn save(config: &AppConfig) -> anyhow::Result<()> {
    let path = config_path()?;
    save_to_path(&path, config)
}

pub fn load_from_path(path: &Path) -> anyhow::Result<AppConfig> {
    if !path.exists() {
        return Ok(AppConfig::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

pub fn save_to_path(path: &Path, config: &AppConfig) -> anyhow::Result<()> {
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

pub fn has_profile_secrets(profile: &S3Profile) -> anyhow::Result<bool> {
    let secrets = load_profile_secrets(profile)?;
    Ok(secrets.access_key_id.is_some() && secrets.secret_access_key.is_some())
}

pub fn delete_profile_secrets(profile: &S3Profile) -> anyhow::Result<()> {
    delete_secret(&profile.access_key_id_ref)?;
    delete_secret(&profile.secret_access_key_ref)?;
    Ok(())
}

fn config_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

fn credentials_path() -> anyhow::Result<PathBuf> {
    Ok(config_dir()?.join("credentials.toml"))
}

fn config_dir() -> anyhow::Result<PathBuf> {
    if let Ok(path) = std::env::var(CONFIG_DIR_ENV) {
        return Ok(PathBuf::from(path));
    }
    let dirs = ProjectDirs::from("org", "henriksson-lab", "img2omezarr")
        .ok_or_else(|| anyhow::anyhow!("could not determine user config directory"))?;
    Ok(dirs.config_dir().to_path_buf())
}

fn access_key_id_ref(profile_name: &str) -> String {
    format!("s3:{profile_name}:access_key_id")
}

fn secret_access_key_ref(profile_name: &str) -> String {
    format!("s3:{profile_name}:secret_access_key")
}

fn save_or_delete_secret(reference: &str, value: Option<&str>) -> anyhow::Result<()> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    let path = credentials_path()?;
    let mut credentials = load_credentials_from_path(&path)?;
    if let Some(value) = value {
        credentials
            .secrets
            .insert(reference.to_string(), value.to_string());
    } else {
        credentials.secrets.remove(reference);
    }
    save_credentials_to_path(&path, &credentials)?;
    Ok(())
}

fn load_secret(reference: &str) -> anyhow::Result<Option<String>> {
    let credentials = load_credentials_from_path(&credentials_path()?)?;
    Ok(credentials.secrets.get(reference).cloned())
}

fn delete_secret(reference: &str) -> anyhow::Result<()> {
    let path = credentials_path()?;
    let mut credentials = load_credentials_from_path(&path)?;
    credentials.secrets.remove(reference);
    save_credentials_to_path(&path, &credentials)?;
    Ok(())
}

fn load_credentials_from_path(path: &Path) -> anyhow::Result<CredentialsFile> {
    if !path.exists() {
        return Ok(CredentialsFile::default());
    }
    let text = fs::read_to_string(path)?;
    Ok(toml::from_str(&text)?)
}

fn save_credentials_to_path(path: &Path, credentials: &CredentialsFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(credentials)?)?;
    restrict_owner_read_write(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_owner_read_write(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_owner_read_write(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn default_ngff_version() -> String {
    "0.5".to_string()
}

fn default_tile_width() -> i32 {
    1024
}

fn default_tile_height() -> i32 {
    1024
}

fn default_chunk_depth() -> i32 {
    1
}

fn default_target_min_size() -> i32 {
    256
}

fn default_downsampling() -> String {
    "nearest".to_string()
}

fn default_compression() -> String {
    "zstd".to_string()
}

fn default_compression_level() -> i32 {
    3
}

fn default_write_omero() -> bool {
    true
}

fn default_upload_mode() -> String {
    "local".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn serialized_config_does_not_store_secret_values() {
        let config = AppConfig {
            default_profile: Some("lab".to_string()),
            settings: GuiSettings {
                upload_mode: "s3".to_string(),
                ..GuiSettings::default()
            },
            s3_profiles: vec![profile_for_fields(
                "lab".to_string(),
                "bucket".to_string(),
                "prefix".to_string(),
                Some("us-east-1".to_string()),
                Some("https://minio.example.org".to_string()),
            )],
        };

        let text = toml::to_string(&config).expect("serialize config");
        assert!(text.contains("access_key_id_ref"));
        assert!(text.contains("secret_access_key_ref"));
        assert!(!text.contains("AKIA"));
        assert!(!text.contains("secret-value"));
    }

    #[test]
    fn profile_metadata_persists_without_credentials() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.toml");
        let config = AppConfig {
            default_profile: Some("lab".to_string()),
            s3_profiles: vec![profile_for_fields(
                "lab".to_string(),
                "bucket".to_string(),
                "prefix".to_string(),
                Some("us-east-1".to_string()),
                Some("http://127.0.0.1:9000".to_string()),
            )],
            ..AppConfig::default()
        };

        save_to_path(&path, &config).expect("save config");
        let loaded = load_from_path(&path).expect("load config");

        let profile = loaded.find_profile("lab").expect("profile persisted");
        assert_eq!(loaded.default_profile.as_deref(), Some("lab"));
        assert_eq!(profile.bucket, "bucket");
        assert_eq!(profile.prefix, "prefix");
        assert_eq!(profile.region.as_deref(), Some("us-east-1"));
        assert_eq!(profile.endpoint.as_deref(), Some("http://127.0.0.1:9000"));
        let text = fs::read_to_string(path).expect("read config");
        assert!(!text.contains("access-secret"));
        assert!(!text.contains("secret-secret"));
    }

    #[test]
    fn profile_credentials_persist_in_separate_file_and_delete_cleanly() {
        let _guard = env_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var(CONFIG_DIR_ENV, temp.path());

        let profile = profile_for_fields(
            "lab".to_string(),
            "bucket".to_string(),
            "prefix".to_string(),
            None,
            None,
        );

        save_profile_secrets(&profile, Some("access-secret"), Some("secret-secret"))
            .expect("save profile secrets");
        let secrets = load_profile_secrets(&profile).expect("load profile secrets");
        assert_eq!(secrets.access_key_id.as_deref(), Some("access-secret"));
        assert_eq!(secrets.secret_access_key.as_deref(), Some("secret-secret"));
        assert!(has_profile_secrets(&profile).expect("has profile secrets"));

        let config_text = toml::to_string(&AppConfig {
            s3_profiles: vec![profile.clone()],
            ..AppConfig::default()
        })
        .expect("serialize config");
        assert!(!config_text.contains("access-secret"));
        assert!(!config_text.contains("secret-secret"));

        let credentials_text =
            fs::read_to_string(temp.path().join("credentials.toml")).expect("read credentials");
        assert!(credentials_text.contains("access-secret"));
        assert!(credentials_text.contains("secret-secret"));

        delete_profile_secrets(&profile).expect("delete profile secrets");
        let secrets = load_profile_secrets(&profile).expect("reload profile secrets");
        assert!(secrets.access_key_id.is_none());
        assert!(secrets.secret_access_key.is_none());
        assert!(!has_profile_secrets(&profile).expect("has profile secrets after delete"));

        std::env::remove_var(CONFIG_DIR_ENV);
    }
}
