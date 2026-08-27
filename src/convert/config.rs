use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ConvertJob {
    pub input: PathBuf,
    pub output: OutputTarget,
}

#[derive(Debug, Clone)]
pub enum OutputTarget {
    Local(PathBuf),
    Upload(UploadTarget),
}

#[derive(Debug, Clone)]
pub enum UploadTarget {
    Local(PathBuf),
    #[cfg(feature = "upload-s3")]
    S3 {
        bucket: String,
        prefix: String,
        region: Option<String>,
        endpoint: Option<String>,
        credentials: Option<S3Credentials>,
    },
}

#[cfg(feature = "upload-s3")]
#[derive(Debug, Clone)]
pub struct S3Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NgffVersion {
    V05,
    V04,
}

impl Default for NgffVersion {
    fn default() -> Self {
        Self::V05
    }
}

impl NgffVersion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::V05 => "0.5",
            Self::V04 => "0.4",
        }
    }
}

#[derive(Debug, Clone)]
pub enum SeriesSelection {
    All,
    Indices(Vec<usize>),
}

impl Default for SeriesSelection {
    fn default() -> Self {
        Self::All
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionPolicy {
    TargetMinSize(u32),
    ExplicitLevels(usize),
    ExistingOrTargetMinSize(u32),
}

impl Default for ResolutionPolicy {
    fn default() -> Self {
        Self::TargetMinSize(256)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Downsampling {
    Nearest,
    Average,
}

impl Default for Downsampling {
    fn default() -> Self {
        Self::Nearest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionCodec {
    Zstd,
    Blosc,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionSettings {
    pub codec: CompressionCodec,
    pub level: Option<i32>,
}

impl Default for CompressionSettings {
    fn default() -> Self {
        Self {
            codec: CompressionCodec::Zstd,
            level: Some(3),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConversionSettings {
    pub ngff_version: NgffVersion,
    pub series: SeriesSelection,
    pub tile_width: usize,
    pub tile_height: usize,
    pub chunk_depth: usize,
    pub resolution_policy: ResolutionPolicy,
    pub downsampling: Downsampling,
    pub compression: CompressionSettings,
    pub overwrite: bool,
    pub write_omero_metadata: bool,
    pub write_ome_xml: bool,
    pub max_workers: usize,
}

impl Default for ConversionSettings {
    fn default() -> Self {
        Self {
            ngff_version: NgffVersion::V05,
            series: SeriesSelection::All,
            tile_width: 1024,
            tile_height: 1024,
            chunk_depth: 1,
            resolution_policy: ResolutionPolicy::TargetMinSize(256),
            downsampling: Downsampling::Nearest,
            compression: CompressionSettings::default(),
            overwrite: false,
            write_omero_metadata: true,
            write_ome_xml: true,
            max_workers: std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
        }
    }
}
