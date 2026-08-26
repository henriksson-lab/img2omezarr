mod run;

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, Downsampling,
    NgffVersion, OutputTarget, ResolutionPolicy, SeriesSelection, UploadTarget,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    output_dir: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    let window = AppWindow::new()?;
    let state = Rc::new(RefCell::new(State::default()));

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_add_files(move || {
            let Some(files) = rfd::FileDialog::new().pick_files() else {
                return;
            };
            add_files(&window_weak, &state, files);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_files_dropped(move |data| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            let text = match data.plain_text() {
                Ok(text) => text,
                Err(err) => {
                    append_log(&window, &format!("drop did not contain file paths: {err}"));
                    return;
                }
            };
            let files = dropped_paths(text.as_str());
            if files.is_empty() {
                append_log(&window, "drop did not contain file paths");
                return;
            }
            drop(window);
            add_files(&window_weak, &state, files);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_remove_file(move |index| {
            let mut state_mut = state.borrow_mut();
            let index = index as usize;
            if index < state_mut.files.len() {
                state_mut.files.remove(index);
            }
            drop(state_mut);
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_move_file(move |index, delta| {
            let mut state_mut = state.borrow_mut();
            let index = index as usize;
            let next = match delta.cmp(&0) {
                std::cmp::Ordering::Less => index.checked_sub(1),
                std::cmp::Ordering::Greater => index.checked_add(1),
                std::cmp::Ordering::Equal => Some(index),
            };
            if let Some(next) = next {
                if index < state_mut.files.len() && next < state_mut.files.len() {
                    state_mut.files.swap(index, next);
                }
            }
            drop(state_mut);
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_pick_output(move || {
            let Some(dir) = rfd::FileDialog::new().pick_folder() else {
                return;
            };
            state.borrow_mut().output_dir = Some(dir.clone());
            if let Some(window) = window_weak.upgrade() {
                window.set_output_dir(SharedString::from(dir.display().to_string()));
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_start_upload(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if window.get_running() {
                return;
            }
            let settings = settings_from_window(&window);
            let staged_upload = window.get_staged_upload();
            let target_settings = match target_settings_from_window(&window, &state) {
                Ok(target_settings) => target_settings,
                Err(err) => {
                    append_log(&window, &err.to_string());
                    return;
                }
            };
            let files = state.borrow().files.clone();
            let jobs = match files
                .into_iter()
                .map(|input| {
                    let stem = input
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("image")
                        .to_string();
                    Ok(ConvertJob {
                        input,
                        output: output_target(&target_settings, &stem, staged_upload)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
            {
                Ok(jobs) => jobs,
                Err(err) => {
                    append_log(&window, &err.to_string());
                    return;
                }
            };
            if jobs.is_empty() {
                append_log(&window, "add at least one image first");
                return;
            }
            refresh_files(&window_weak, &state);
            window.set_running(true);
            run::start_conversion(window_weak.clone(), jobs, settings);
        });
    }

    window.run()?;
    Ok(())
}

enum GuiTargetSettings {
    Local(PathBuf),
    S3 {
        bucket: String,
        prefix: String,
        region: Option<String>,
        endpoint: Option<String>,
    },
}

fn target_settings_from_window(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
) -> anyhow::Result<GuiTargetSettings> {
    if window.get_upload_mode() == SharedString::from("s3") {
        let bucket = window.get_s3_bucket().trim().to_string();
        let prefix = window.get_s3_prefix().trim().trim_matches('/').to_string();
        if bucket.is_empty() || prefix.is_empty() {
            anyhow::bail!("enter both S3 bucket and S3 prefix");
        }
        return Ok(GuiTargetSettings::S3 {
            bucket,
            prefix,
            region: optional_string(window.get_s3_region()),
            endpoint: optional_string(window.get_s3_endpoint()),
        });
    }

    let Some(path) = state.borrow().output_dir.clone() else {
        anyhow::bail!("choose an output folder first");
    };
    Ok(GuiTargetSettings::Local(path))
}

fn output_target(
    target: &GuiTargetSettings,
    stem: &str,
    staged_upload: bool,
) -> anyhow::Result<OutputTarget> {
    match target {
        GuiTargetSettings::Local(output_dir) => {
            let path = output_dir.join(format!("{stem}.ome.zarr"));
            if staged_upload {
                Ok(OutputTarget::Upload(UploadTarget::Local(path)))
            } else {
                Ok(OutputTarget::Local(path))
            }
        }
        GuiTargetSettings::S3 {
            bucket,
            prefix,
            region,
            endpoint,
        } => s3_output_target(bucket, prefix, stem, region, endpoint),
    }
}

#[cfg(feature = "upload-s3")]
fn s3_output_target(
    bucket: &str,
    prefix: &str,
    stem: &str,
    region: &Option<String>,
    endpoint: &Option<String>,
) -> anyhow::Result<OutputTarget> {
    let prefix = format!("{}/{}.ome.zarr", prefix.trim_matches('/'), stem);
    Ok(OutputTarget::Upload(UploadTarget::S3 {
        bucket: bucket.to_string(),
        prefix,
        region: region.clone(),
        endpoint: endpoint.clone(),
        profile: None,
    }))
}

#[cfg(not(feature = "upload-s3"))]
fn s3_output_target(
    _bucket: &str,
    _prefix: &str,
    _stem: &str,
    _region: &Option<String>,
    _endpoint: &Option<String>,
) -> anyhow::Result<OutputTarget> {
    anyhow::bail!("S3 upload requires building the GUI with --features upload-s3")
}

fn optional_string(value: SharedString) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn settings_from_window(window: &AppWindow) -> ConversionSettings {
    ConversionSettings {
        ngff_version: if window.get_ngff_version() == SharedString::from("0.4") {
            NgffVersion::V04
        } else {
            NgffVersion::V05
        },
        series: SeriesSelection::All,
        tile_width: window.get_tile_width().max(1) as usize,
        tile_height: window.get_tile_height().max(1) as usize,
        chunk_depth: window.get_chunk_depth().max(1) as usize,
        resolution_policy: if window.get_use_existing_resolutions() {
            ResolutionPolicy::ExistingOrTargetMinSize(window.get_target_min_size().max(1) as u32)
        } else {
            ResolutionPolicy::TargetMinSize(window.get_target_min_size().max(1) as u32)
        },
        downsampling: if window.get_downsampling() == SharedString::from("average") {
            Downsampling::Average
        } else {
            Downsampling::Nearest
        },
        compression: CompressionSettings {
            codec: match window.get_compression().as_str() {
                "none" => CompressionCodec::None,
                "blosc" => CompressionCodec::Blosc,
                _ => CompressionCodec::Zstd,
            },
            level: if window.get_compression() == SharedString::from("none") {
                None
            } else {
                Some(window.get_compression_level())
            },
        },
        overwrite: window.get_overwrite(),
        write_omero_metadata: window.get_write_omero(),
        write_ome_xml: true,
        max_workers: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        ..ConversionSettings::default()
    }
}

fn refresh_files(window: &slint::Weak<AppWindow>, state: &Rc<RefCell<State>>) {
    let Some(window) = window.upgrade() else {
        return;
    };
    let rows = state
        .borrow()
        .files
        .iter()
        .map(|path| QueueFile {
            path: SharedString::from(path.display().to_string()),
            status: SharedString::from("queued"),
        })
        .collect::<Vec<_>>();
    window.set_files(ModelRc::new(VecModel::from(rows)));
}

fn add_files(window: &slint::Weak<AppWindow>, state: &Rc<RefCell<State>>, files: Vec<PathBuf>) {
    {
        let mut state = state.borrow_mut();
        state.files.extend(files);
    }
    refresh_files(window, state);
}

fn append_log(window: &AppWindow, line: &str) {
    let current = window.get_log_text();
    let next = if current.is_empty() {
        line.to_string()
    } else {
        format!("{current}\n{line}")
    };
    window.set_log_text(SharedString::from(next));
}

fn dropped_paths(payload: &str) -> Vec<PathBuf> {
    payload
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            if line.starts_with("file://") {
                decode_file_uri(line).map(PathBuf::from)
            } else {
                Some(PathBuf::from(line))
            }
        })
        .collect()
}

fn decode_file_uri(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://")?;
    let path = path.strip_prefix("localhost/").unwrap_or(path);
    let decoded = percent_decode(path)?;
    #[cfg(windows)]
    {
        if decoded.len() >= 3 && decoded.as_bytes()[0] == b'/' && decoded.as_bytes()[2] == b':' {
            return Some(decoded[1..].to_string());
        }
    }
    Some(if decoded.starts_with('/') {
        decoded
    } else {
        format!("/{decoded}")
    })
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            out.push((high << 4) | low);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_paths_accepts_uri_list() {
        let paths =
            dropped_paths("# comment\nfile:///tmp/a%20b.ome.tif\r\nfile://localhost/tmp/c.tif\n");
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/tmp/a b.ome.tif"),
                PathBuf::from("/tmp/c.tif")
            ]
        );
    }

    #[test]
    fn dropped_paths_accepts_plain_paths() {
        let paths = dropped_paths("/tmp/a.tif\nrelative.nd2\n");
        assert_eq!(
            paths,
            vec![PathBuf::from("/tmp/a.tif"), PathBuf::from("relative.nd2")]
        );
    }

    #[test]
    fn bad_file_uri_is_ignored() {
        let paths = dropped_paths("file:///tmp/%zz.tif\n/tmp/good.tif\n");
        assert_eq!(paths, vec![PathBuf::from("/tmp/good.tif")]);
    }
}
