use std::fs;
use std::path::{Path, PathBuf};

use crate::convert::config::{OutputTarget, UploadTarget};

#[cfg(feature = "upload-s3")]
use object_store::aws::AmazonS3Builder;
#[cfg(feature = "upload-s3")]
use object_store::{path::Path as ObjectPath, ObjectStore};

pub fn materialize_output_target(target: &OutputTarget) -> anyhow::Result<PathBuf> {
    match target {
        OutputTarget::Local(path) => Ok(path.clone()),
        OutputTarget::Upload(UploadTarget::Local(_)) => temp_output_path(),
        #[cfg(feature = "upload-s3")]
        OutputTarget::Upload(UploadTarget::S3 { .. }) => temp_output_path(),
    }
}

pub fn final_output_path(target: &OutputTarget) -> PathBuf {
    match target {
        OutputTarget::Local(path) => path.clone(),
        OutputTarget::Upload(UploadTarget::Local(path)) => path.clone(),
        #[cfg(feature = "upload-s3")]
        OutputTarget::Upload(UploadTarget::S3 { bucket, prefix, .. }) => {
            PathBuf::from(format!("s3://{bucket}/{prefix}"))
        }
    }
}

pub fn finish_output_target(local_output: &Path, target: &OutputTarget) -> anyhow::Result<()> {
    match target {
        OutputTarget::Local(_) => Ok(()),
        OutputTarget::Upload(UploadTarget::Local(path)) => {
            if path.exists() {
                if path.is_dir() {
                    fs::remove_dir_all(path)?;
                } else {
                    fs::remove_file(path)?;
                }
            }
            copy_dir_all(local_output, path)?;
            fs::remove_dir_all(local_output).ok();
            Ok(())
        }
        #[cfg(feature = "upload-s3")]
        OutputTarget::Upload(UploadTarget::S3 {
            bucket,
            prefix,
            region,
            endpoint,
            credentials,
        }) => {
            upload_dir_to_s3(local_output, bucket, prefix, region, endpoint, credentials)?;
            fs::remove_dir_all(local_output).ok();
            Ok(())
        }
    }
}

#[cfg(feature = "upload-s3")]
pub fn test_s3_connection(
    bucket: &str,
    prefix: &str,
    region: &Option<String>,
    endpoint: &Option<String>,
    credentials: &Option<crate::convert::config::S3Credentials>,
) -> anyhow::Result<()> {
    let store = s3_store(bucket, region, endpoint, credentials)?;
    let runtime = tokio::runtime::Runtime::new()?;
    let prefix = prefix.trim_matches('/');
    let prefix = if prefix.is_empty() {
        None
    } else {
        Some(ObjectPath::from(prefix))
    };
    runtime.block_on(store.list_with_delimiter(prefix.as_ref()))?;
    Ok(())
}

fn temp_output_path() -> anyhow::Result<PathBuf> {
    Ok(std::env::temp_dir().join(format!(
        "img2omezarr-{}-{}.ome.zarr",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    )))
}

fn copy_dir_all(from: &Path, to: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst = to.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), dst)?;
        }
    }
    Ok(())
}

#[cfg(feature = "upload-s3")]
fn upload_dir_to_s3(
    local_output: &Path,
    bucket: &str,
    prefix: &str,
    region: &Option<String>,
    endpoint: &Option<String>,
    credentials: &Option<crate::convert::config::S3Credentials>,
) -> anyhow::Result<()> {
    let store = s3_store(bucket, region, endpoint, credentials)?;
    let runtime = tokio::runtime::Runtime::new()?;
    for file in list_files(local_output)? {
        let relative = file.strip_prefix(local_output)?;
        let key = join_object_key(prefix, relative);
        let bytes = fs::read(&file)?;
        runtime.block_on(store.put(&ObjectPath::from(key), bytes.into()))?;
    }
    Ok(())
}

#[cfg(feature = "upload-s3")]
fn s3_store(
    bucket: &str,
    region: &Option<String>,
    endpoint: &Option<String>,
    credentials: &Option<crate::convert::config::S3Credentials>,
) -> anyhow::Result<impl ObjectStore> {
    let mut builder = AmazonS3Builder::from_env().with_bucket_name(bucket);
    if let Some(region) = region {
        builder = builder.with_region(region);
    }
    if let Some(endpoint) = endpoint {
        builder = builder.with_endpoint(endpoint);
        if endpoint.starts_with("http://") {
            builder = builder.with_allow_http(true);
        }
    }
    if let Some(credentials) = credentials {
        builder = builder
            .with_access_key_id(&credentials.access_key_id)
            .with_secret_access_key(&credentials.secret_access_key);
        if let Some(token) = &credentials.session_token {
            builder = builder.with_token(token);
        }
    }

    Ok(builder.build()?)
}

#[cfg(feature = "upload-s3")]
fn list_files(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    Ok(files)
}

#[cfg(feature = "upload-s3")]
fn collect_files(path: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_files(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(feature = "upload-s3")]
fn join_object_key(prefix: &str, relative: &Path) -> String {
    let mut key = prefix.trim_matches('/').to_string();
    for component in relative.components() {
        let part = component.as_os_str().to_string_lossy();
        if part.is_empty() {
            continue;
        }
        if !key.is_empty() {
            key.push('/');
        }
        key.push_str(&part);
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_upload_uses_temporary_materialization() {
        let target = OutputTarget::Upload(UploadTarget::Local(PathBuf::from("final.ome.zarr")));
        let materialized = materialize_output_target(&target).unwrap();
        assert_ne!(materialized, PathBuf::from("final.ome.zarr"));
        assert_eq!(final_output_path(&target), PathBuf::from("final.ome.zarr"));
    }

    #[cfg(feature = "upload-s3")]
    #[test]
    fn s3_keys_join_prefix_and_relative_path() {
        assert_eq!(
            join_object_key("dataset.ome.zarr", Path::new("0/0/zarr.json")),
            "dataset.ome.zarr/0/0/zarr.json"
        );
        assert_eq!(join_object_key("", Path::new("zarr.json")), "zarr.json");
    }
}
