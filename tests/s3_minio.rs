#![cfg(all(feature = "cli", feature = "upload-s3"))]

use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::TryStreamExt;
use object_store::aws::{AmazonS3, AmazonS3Builder};
use object_store::path::Path as ObjectPath;
use object_store::ObjectStore;
use zarrs::array::{Array as ZarrArray, ArraySubset};
use zarrs::filesystem::FilesystemStore;

#[test]
fn cli_uploads_readable_omezarr_to_s3_compatible_store() {
    let Some(config) = MinioConfig::from_env() else {
        eprintln!("skipping S3 integration test; set IMG2OMEZARR_S3_TEST_ENDPOINT, IMG2OMEZARR_S3_TEST_BUCKET, AWS_ACCESS_KEY_ID, and AWS_SECRET_ACCESS_KEY");
        return;
    };

    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("s3&sizeX=4&sizeY=4&sizeZ=1&sizeC=1.fake");
    fs::write(&input, b"").expect("write fake input");

    let prefix = format!("img2omezarr-test-{}", unique_suffix());
    let output = format!("s3://{}/{}/dataset.ome.zarr", config.bucket, prefix);
    let local_output = temp.path().join("local.ome.zarr");

    let local_conversion = local_conversion_command(&input, &local_output)
        .arg("--overwrite")
        .output()
        .expect("run local img2omezarr");
    assert!(
        local_conversion.status.success(),
        "local conversion command failed: {}",
        String::from_utf8_lossy(&local_conversion.stderr)
    );

    let first_upload = conversion_command(&config, &input, &output)
        .arg("--overwrite")
        .output()
        .expect("run img2omezarr");
    assert!(
        first_upload.status.success(),
        "CLI upload command failed: {}",
        String::from_utf8_lossy(&first_upload.stderr)
    );

    let duplicate_upload = conversion_command(&config, &input, &output)
        .output()
        .expect("run img2omezarr");
    assert!(
        !duplicate_upload.status.success(),
        "duplicate upload without --overwrite unexpectedly succeeded"
    );
    assert!(
        String::from_utf8_lossy(&duplicate_upload.stderr).contains("output already exists"),
        "duplicate upload did not report existing output: {}",
        String::from_utf8_lossy(&duplicate_upload.stderr)
    );

    let overwrite_upload = conversion_command(&config, &input, &output)
        .arg("--overwrite")
        .output()
        .expect("run img2omezarr");
    assert!(
        overwrite_upload.status.success(),
        "overwrite upload failed: {}",
        String::from_utf8_lossy(&overwrite_upload.stderr)
    );

    let bad_credentials_output =
        format!("s3://{}/{}/bad-credentials.ome.zarr", config.bucket, prefix);
    let bad_credentials = conversion_command(&config, &input, &bad_credentials_output)
        .env("AWS_ACCESS_KEY_ID", "invalid-access-key")
        .env("AWS_SECRET_ACCESS_KEY", "invalid-secret-key")
        .output()
        .expect("run img2omezarr");
    assert!(
        !bad_credentials.status.success(),
        "upload with invalid credentials unexpectedly succeeded"
    );

    let store = AmazonS3Builder::from_env()
        .with_bucket_name(&config.bucket)
        .with_region(&config.region)
        .with_endpoint(&config.endpoint)
        .with_access_key_id(&config.access_key_id)
        .with_secret_access_key(&config.secret_access_key)
        .with_allow_http(config.endpoint.starts_with("http://"))
        .build()
        .expect("build S3 store");
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    let dataset_prefix = format!("{prefix}/dataset.ome.zarr");
    for key in [
        format!("{dataset_prefix}/zarr.json"),
        format!("{dataset_prefix}/0/zarr.json"),
        format!("{dataset_prefix}/0/0/zarr.json"),
    ] {
        runtime
            .block_on(store.head(&ObjectPath::from(key.clone())))
            .unwrap_or_else(|err| panic!("missing uploaded object {key}: {err}"));
    }

    let image_group = runtime
        .block_on(store.get(&ObjectPath::from(format!("{dataset_prefix}/0/zarr.json"))))
        .expect("get image group metadata");
    let bytes = runtime
        .block_on(image_group.bytes())
        .expect("read image group metadata");
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).expect("parse uploaded image group metadata");
    assert_eq!(json["attributes"]["ome"]["version"], "0.5");

    let downloaded_output = temp.path().join("downloaded.ome.zarr");
    download_prefix(&store, &runtime, &dataset_prefix, &downloaded_output);
    assert_eq!(
        read_level0_pixels(&downloaded_output),
        read_level0_pixels(&local_output)
    );
}

fn conversion_command(config: &MinioConfig, input: &Path, output: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_img2omezarr"));
    command
        .arg("convert")
        .arg(input)
        .arg(output)
        .arg("--s3-region")
        .arg(&config.region)
        .arg("--s3-endpoint")
        .arg(&config.endpoint)
        .arg("--tile-width")
        .arg("4")
        .arg("--tile-height")
        .arg("4")
        .arg("--target-min-size")
        .arg("4")
        .arg("--compression")
        .arg("none")
        .env("AWS_ACCESS_KEY_ID", &config.access_key_id)
        .env("AWS_SECRET_ACCESS_KEY", &config.secret_access_key);
    command
}

fn local_conversion_command(input: &Path, output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_img2omezarr"));
    command
        .arg("convert")
        .arg(input)
        .arg(output)
        .arg("--tile-width")
        .arg("4")
        .arg("--tile-height")
        .arg("4")
        .arg("--target-min-size")
        .arg("4")
        .arg("--compression")
        .arg("none");
    command
}

fn download_prefix(store: &AmazonS3, runtime: &tokio::runtime::Runtime, prefix: &str, dest: &Path) {
    fs::create_dir_all(dest).expect("create downloaded output directory");
    runtime
        .block_on(async {
            let prefix_path = ObjectPath::from(prefix.to_string());
            let mut stream = store.list(Some(&prefix_path));
            while let Some(meta) = stream.try_next().await? {
                let key = meta.location.to_string();
                let relative = key
                    .strip_prefix(prefix)
                    .expect("listed key starts with prefix")
                    .trim_start_matches('/');
                let target = dest.join(relative);
                if let Some(parent) = target.parent() {
                    fs::create_dir_all(parent)?;
                }
                let object = store.get(&meta.location).await?;
                let bytes = object.bytes().await?;
                fs::write(target, bytes)?;
            }
            Ok::<_, anyhow::Error>(())
        })
        .expect("download S3 prefix");
}

fn read_level0_pixels(root: &Path) -> Vec<u8> {
    let store = Arc::new(FilesystemStore::new(root).expect("open zarr store"));
    let array = ZarrArray::open(store, "/0/0").expect("open zarr array");
    let subset =
        ArraySubset::new_with_start_shape(vec![0; 5], vec![1, 1, 1, 4, 4]).expect("subset");
    array
        .retrieve_array_subset::<Vec<u8>>(&subset)
        .expect("read zarr pixels")
}

struct MinioConfig {
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
}

impl MinioConfig {
    fn from_env() -> Option<Self> {
        Some(Self {
            endpoint: std::env::var("IMG2OMEZARR_S3_TEST_ENDPOINT").ok()?,
            bucket: std::env::var("IMG2OMEZARR_S3_TEST_BUCKET").ok()?,
            region: std::env::var("AWS_REGION")
                .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
                .unwrap_or_else(|_| "us-east-1".to_string()),
            access_key_id: std::env::var("AWS_ACCESS_KEY_ID").ok()?,
            secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").ok()?,
        })
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos()
}
