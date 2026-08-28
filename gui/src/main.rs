mod config;
mod run;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, Downsampling,
    NgffVersion, OutputTarget, ResolutionPolicy, SeriesSelection, UploadTarget,
};
use img2omezarr::convert::upload::final_output_path;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

slint::include_modules!();

#[derive(Default)]
struct State {
    files: Vec<PathBuf>,
    output_dir: Option<PathBuf>,
    app_config: config::AppConfig,
    cancel_requested: Arc<AtomicBool>,
}

fn main() -> anyhow::Result<()> {
    let window = AppWindow::new()?;
    let app_config = match config::load() {
        Ok(config) => config,
        Err(err) => {
            append_log(&window, &format!("could not load saved config: {err:#}"));
            config::AppConfig::default()
        }
    };
    let state = Rc::new(RefCell::new(State {
        app_config,
        cancel_requested: Arc::new(AtomicBool::new(false)),
        ..State::default()
    }));
    apply_saved_settings(&window, &state);
    refresh_s3_profiles(&window, &state);
    if let Err(err) = load_default_s3_profile(&window, &state) {
        append_log(
            &window,
            &format!("could not load default S3 profile: {err:#}"),
        );
    }

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
                if let Err(err) = persist_current_settings(&window, &state) {
                    append_log(&window, &format!("could not save settings: {err:#}"));
                }
            }
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_target_settings_changed(move || {
            refresh_files(&window_weak, &state);
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
            window.set_error_visible(false);
            window.set_error_message(SharedString::from(""));
            state
                .borrow()
                .cancel_requested
                .store(false, Ordering::SeqCst);
            if let Err(err) = persist_current_settings(&window, &state) {
                show_error(&window, &format!("could not save settings: {err:#}"));
            }
            let settings = settings_from_window(&window);
            let staged_upload = window.get_staged_upload();
            let target_settings = match target_settings_from_window(&window, &state) {
                Ok(target_settings) => target_settings,
                Err(err) => {
                    show_error(&window, &err.to_string());
                    return;
                }
            };
            let files = state.borrow().files.clone();
            let jobs = match files
                .into_iter()
                .map(|input| {
                    let stem = output_stem(&input);
                    Ok(ConvertJob {
                        input,
                        output: output_target(&target_settings, &stem, staged_upload)?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()
            {
                Ok(jobs) => jobs,
                Err(err) => {
                    show_error(&window, &err.to_string());
                    return;
                }
            };
            if let Err(err) = validate_unique_output_targets(&jobs) {
                show_error(&window, &err.to_string());
                return;
            }
            if jobs.is_empty() {
                show_error(&window, "add at least one image first");
                return;
            }
            refresh_files(&window_weak, &state);
            window.set_running(true);
            let cancel_requested = Arc::clone(&state.borrow().cancel_requested);
            run::start_conversion(window_weak.clone(), jobs, settings, cancel_requested);
        });
    }

    {
        let state = Rc::clone(&state);
        let window_weak = window.as_weak();
        window.on_cancel_upload(move || {
            state
                .borrow()
                .cancel_requested
                .store(true, Ordering::SeqCst);
            if let Some(window) = window_weak.upgrade() {
                window.set_progress_label(SharedString::from("cancelling after current file"));
                append_log(&window, "cancelling after current file");
            }
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_dismiss_error(move || {
            if let Some(window) = window_weak.upgrade() {
                window.set_error_visible(false);
                window.set_error_message(SharedString::from(""));
            }
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_s3_profile_selected(move |name| {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if let Err(err) = load_s3_profile(&window, &state, name.as_str()) {
                show_error(&window, &format!("could not load S3 profile: {err:#}"));
            }
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_new_s3_profile(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            clear_s3_profile_fields(&window);
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_save_s3_profile(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            match save_s3_profile(&window, &state) {
                Ok(name) => append_log(&window, &format!("saved S3 profile {name}")),
                Err(err) => show_error(&window, &format!("could not save S3 profile: {err:#}")),
            }
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        let state = Rc::clone(&state);
        window.on_delete_s3_profile(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            match delete_s3_profile(&window, &state) {
                Ok(name) => append_log(&window, &format!("deleted S3 profile {name}")),
                Err(err) => show_error(&window, &format!("could not delete S3 profile: {err:#}")),
            }
            refresh_files(&window_weak, &state);
        });
    }

    {
        let window_weak = window.as_weak();
        window.on_test_s3_profile(move || {
            let Some(window) = window_weak.upgrade() else {
                return;
            };
            if window.get_running() {
                return;
            }
            match s3_settings_from_window(&window) {
                Ok(settings) => {
                    window.set_running(true);
                    window.set_progress(-1.0);
                    window.set_progress_label(SharedString::from("testing S3 connection"));
                    let window_weak = window_weak.clone();
                    std::thread::Builder::new()
                        .name("img2omezarr-s3-test".into())
                        .spawn(move || {
                            let result = test_s3_profile(&settings);
                            let _ = slint::invoke_from_event_loop(move || {
                                if let Some(window) = window_weak.upgrade() {
                                    match result {
                                        Ok(()) => {
                                            window.set_progress_label(SharedString::from(
                                                "S3 connection succeeded",
                                            ));
                                            append_log(&window, "S3 connection succeeded");
                                        }
                                        Err(err) => {
                                            window.set_progress_label(SharedString::from(
                                                "S3 connection failed",
                                            ));
                                            show_error(
                                                &window,
                                                &format!("S3 connection failed: {err:#}"),
                                            );
                                        }
                                    }
                                    window.set_progress(-1.0);
                                    window.set_running(false);
                                }
                            });
                        })
                        .expect("start S3 test thread");
                }
                Err(err) => show_error(&window, &err.to_string()),
            }
        });
    }

    let run_result = window.run();
    if let Err(err) = persist_current_settings(&window, &state) {
        eprintln!("could not save settings: {err:#}");
    }
    run_result?;
    Ok(())
}

enum GuiTargetSettings {
    Local(PathBuf),
    S3 {
        bucket: String,
        prefix: String,
        region: Option<String>,
        endpoint: Option<String>,
        credentials: Option<GuiS3Credentials>,
    },
}

fn target_settings_from_window(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
) -> anyhow::Result<GuiTargetSettings> {
    if window.get_upload_mode() == SharedString::from("s3") {
        let settings = s3_settings_from_window(window)?;
        return Ok(GuiTargetSettings::S3 {
            bucket: settings.bucket,
            prefix: settings.prefix,
            region: settings.region,
            endpoint: settings.endpoint,
            credentials: settings.credentials,
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
            credentials,
        } => s3_output_target(bucket, prefix, stem, region, endpoint, credentials),
    }
}

fn validate_unique_output_targets(jobs: &[ConvertJob]) -> anyhow::Result<()> {
    let mut seen = BTreeMap::new();
    for job in jobs {
        let output = final_output_path(&job.output).display().to_string();
        let input = job.input.display().to_string();
        if let Some(previous_input) = seen.insert(output.clone(), input.clone()) {
            anyhow::bail!(
                "multiple inputs would write to the same output target: {previous_input} and {input} -> {output}"
            );
        }
    }
    Ok(())
}

fn output_stem(input: &Path) -> String {
    input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("image")
        .to_string()
}

fn apply_saved_settings(window: &AppWindow, state: &Rc<RefCell<State>>) {
    let settings = state.borrow().app_config.settings.clone();
    window.set_ngff_version(SharedString::from(settings.ngff_version));
    window.set_tile_width(settings.tile_width.max(1));
    window.set_tile_height(settings.tile_height.max(1));
    window.set_chunk_depth(settings.chunk_depth.max(1));
    window.set_target_min_size(settings.target_min_size.max(1));
    window.set_use_existing_resolutions(settings.use_existing_resolutions);
    window.set_downsampling(SharedString::from(settings.downsampling));
    window.set_compression(SharedString::from(settings.compression));
    window.set_compression_level(settings.compression_level);
    window.set_overwrite(settings.overwrite);
    window.set_write_omero(settings.write_omero);
    window.set_staged_upload(settings.staged_upload);
    window.set_upload_mode(SharedString::from(settings.upload_mode));
    if let Some(output_dir) = settings.last_output_dir {
        state.borrow_mut().output_dir = Some(PathBuf::from(&output_dir));
        window.set_output_dir(SharedString::from(output_dir));
    }
}

fn persist_current_settings(window: &AppWindow, state: &Rc<RefCell<State>>) -> anyhow::Result<()> {
    {
        let mut state = state.borrow_mut();
        state.app_config.settings = config::GuiSettings {
            ngff_version: window.get_ngff_version().to_string(),
            tile_width: window.get_tile_width(),
            tile_height: window.get_tile_height(),
            chunk_depth: window.get_chunk_depth(),
            target_min_size: window.get_target_min_size(),
            use_existing_resolutions: window.get_use_existing_resolutions(),
            downsampling: window.get_downsampling().to_string(),
            compression: window.get_compression().to_string(),
            compression_level: window.get_compression_level(),
            overwrite: window.get_overwrite(),
            write_omero: window.get_write_omero(),
            staged_upload: window.get_staged_upload(),
            upload_mode: window.get_upload_mode().to_string(),
            last_output_dir: state
                .output_dir
                .as_ref()
                .map(|path| path.display().to_string()),
        };
        config::save(&state.app_config)?;
    }
    Ok(())
}

fn refresh_s3_profiles(window: &AppWindow, state: &Rc<RefCell<State>>) {
    let names = state.borrow().app_config.profile_names();
    let rows = names
        .into_iter()
        .map(SharedString::from)
        .collect::<Vec<_>>();
    window.set_s3_profile_names(ModelRc::new(VecModel::from(rows)));
}

fn load_default_s3_profile(window: &AppWindow, state: &Rc<RefCell<State>>) -> anyhow::Result<()> {
    let name = {
        let state = state.borrow();
        state.app_config.default_profile.clone().or_else(|| {
            state
                .app_config
                .s3_profiles
                .first()
                .map(|profile| profile.name.clone())
        })
    };
    if let Some(name) = name {
        load_s3_profile(window, state, &name)?;
    }
    Ok(())
}

fn load_s3_profile(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    name: &str,
) -> anyhow::Result<()> {
    if name.trim().is_empty() {
        return Ok(());
    }
    let profile = {
        let state = state.borrow();
        state
            .app_config
            .find_profile(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("S3 profile not found: {name}"))?
    };
    let secrets = config::load_profile_secrets(&profile)?;
    let has_saved_credentials =
        secrets.access_key_id.is_some() && secrets.secret_access_key.is_some();
    window.set_s3_profile(SharedString::from(profile.name));
    window.set_s3_bucket(SharedString::from(profile.bucket));
    window.set_s3_prefix(SharedString::from(profile.prefix));
    window.set_s3_region(SharedString::from(profile.region.unwrap_or_default()));
    window.set_s3_endpoint(SharedString::from(profile.endpoint.unwrap_or_default()));
    window.set_s3_access_key_id(SharedString::from(
        secrets.access_key_id.unwrap_or_default(),
    ));
    window.set_s3_secret_access_key(SharedString::from(
        secrets.secret_access_key.unwrap_or_default(),
    ));
    window.set_s3_has_saved_credentials(has_saved_credentials);
    window.set_s3_clear_credentials(false);
    Ok(())
}

fn clear_s3_profile_fields(window: &AppWindow) {
    window.set_s3_profile(SharedString::from(""));
    window.set_s3_bucket(SharedString::from(""));
    window.set_s3_prefix(SharedString::from(""));
    window.set_s3_region(SharedString::from(""));
    window.set_s3_endpoint(SharedString::from(""));
    window.set_s3_access_key_id(SharedString::from(""));
    window.set_s3_secret_access_key(SharedString::from(""));
    window.set_s3_has_saved_credentials(false);
    window.set_s3_clear_credentials(false);
}

fn save_s3_profile(window: &AppWindow, state: &Rc<RefCell<State>>) -> anyhow::Result<String> {
    let name = window.get_s3_profile().trim().to_string();
    if name.is_empty() {
        anyhow::bail!("enter an S3 profile name");
    }
    let bucket = window.get_s3_bucket().trim().to_string();
    let prefix = window.get_s3_prefix().trim().trim_matches('/').to_string();
    if bucket.is_empty() || prefix.is_empty() {
        anyhow::bail!("enter both S3 bucket and S3 prefix");
    }

    let profile = config::profile_for_fields(
        name.clone(),
        bucket,
        prefix,
        optional_string(window.get_s3_region()),
        optional_string(window.get_s3_endpoint()),
    );
    let access_key_id = optional_string(window.get_s3_access_key_id());
    let secret_access_key = optional_string(window.get_s3_secret_access_key());
    match (
        window.get_s3_clear_credentials(),
        access_key_id.as_deref(),
        secret_access_key.as_deref(),
    ) {
        (true, _, _) => config::delete_profile_secrets(&profile)?,
        (false, Some(access_key_id), Some(secret_access_key)) => {
            config::save_profile_secrets(&profile, Some(access_key_id), Some(secret_access_key))?;
        }
        (false, None, None) => {}
        (false, _, _) => {
            anyhow::bail!(
                "enter both S3 access key ID and secret access key, or leave both empty to keep existing saved credentials"
            )
        }
    }

    {
        let mut state = state.borrow_mut();
        state.app_config.default_profile = Some(name.clone());
        state.app_config.upsert_profile(profile);
        config::save(&state.app_config)?;
    }
    refresh_s3_profiles(window, state);
    window.set_s3_profile(SharedString::from(name.clone()));
    let has_saved_credentials = {
        let state = state.borrow();
        state
            .app_config
            .find_profile(&name)
            .map(config::has_profile_secrets)
            .transpose()?
            .unwrap_or(false)
    };
    window.set_s3_has_saved_credentials(has_saved_credentials);
    window.set_s3_clear_credentials(false);
    Ok(name)
}

fn delete_s3_profile(window: &AppWindow, state: &Rc<RefCell<State>>) -> anyhow::Result<String> {
    let name = window.get_s3_profile().trim().to_string();
    if name.is_empty() {
        anyhow::bail!("select or enter an S3 profile name");
    }
    let removed = {
        let mut state = state.borrow_mut();
        let removed = state
            .app_config
            .remove_profile(&name)
            .ok_or_else(|| anyhow::anyhow!("S3 profile not found: {name}"))?;
        if state.app_config.default_profile.as_deref() == Some(&name) {
            state.app_config.default_profile = state
                .app_config
                .s3_profiles
                .first()
                .map(|profile| profile.name.clone());
        }
        config::save(&state.app_config)?;
        removed
    };
    config::delete_profile_secrets(&removed)?;
    refresh_s3_profiles(window, state);
    clear_s3_profile_fields(window);
    load_default_s3_profile(window, state)?;
    Ok(name)
}

#[derive(Clone, Debug)]
struct GuiS3Settings {
    bucket: String,
    prefix: String,
    region: Option<String>,
    endpoint: Option<String>,
    credentials: Option<GuiS3Credentials>,
}

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "upload-s3"), allow(dead_code))]
struct GuiS3Credentials {
    access_key_id: String,
    secret_access_key: String,
}

fn s3_settings_from_window(window: &AppWindow) -> anyhow::Result<GuiS3Settings> {
    s3_settings_from_fields(
        window.get_s3_profile().as_str(),
        window.get_s3_bucket().as_str(),
        window.get_s3_prefix().as_str(),
        window.get_s3_region().as_str(),
        window.get_s3_endpoint().as_str(),
        window.get_s3_access_key_id().as_str(),
        window.get_s3_secret_access_key().as_str(),
    )
}

fn s3_settings_from_fields(
    profile_name: &str,
    bucket: &str,
    prefix: &str,
    region: &str,
    endpoint: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> anyhow::Result<GuiS3Settings> {
    let bucket = bucket.trim().to_string();
    let prefix = prefix.trim().trim_matches('/').to_string();
    if bucket.is_empty() || prefix.is_empty() {
        anyhow::bail!("enter both S3 bucket and S3 prefix");
    }
    let access_key_id = optional_str(access_key_id);
    let secret_access_key = optional_str(secret_access_key);
    let credentials = match (access_key_id, secret_access_key) {
        (Some(access_key_id), Some(secret_access_key)) => Some(GuiS3Credentials {
            access_key_id,
            secret_access_key,
        }),
        (None, None) => saved_profile_credentials_for_name(profile_name)?,
        _ => {
            anyhow::bail!("enter both S3 access key ID and secret access key, or leave both empty")
        }
    };
    Ok(GuiS3Settings {
        bucket,
        prefix,
        region: optional_str(region),
        endpoint: optional_str(endpoint),
        credentials,
    })
}

fn saved_profile_credentials_for_name(
    profile_name: &str,
) -> anyhow::Result<Option<GuiS3Credentials>> {
    let profile_name = profile_name.trim().to_string();
    if profile_name.is_empty() {
        return Ok(None);
    }
    let app_config = config::load()?;
    let Some(profile) = app_config.find_profile(&profile_name) else {
        return Ok(None);
    };
    let secrets = config::load_profile_secrets(profile)?;
    match (secrets.access_key_id, secrets.secret_access_key) {
        (Some(access_key_id), Some(secret_access_key)) => Ok(Some(GuiS3Credentials {
            access_key_id,
            secret_access_key,
        })),
        (None, None) => Ok(None),
        _ => anyhow::bail!("saved S3 profile credentials are incomplete"),
    }
}

#[cfg(feature = "upload-s3")]
fn s3_output_target(
    bucket: &str,
    prefix: &str,
    stem: &str,
    region: &Option<String>,
    endpoint: &Option<String>,
    credentials: &Option<GuiS3Credentials>,
) -> anyhow::Result<OutputTarget> {
    let prefix = format!("{}/{}.ome.zarr", prefix.trim_matches('/'), stem);
    Ok(OutputTarget::Upload(UploadTarget::S3 {
        bucket: bucket.to_string(),
        prefix,
        region: region.clone(),
        endpoint: endpoint.clone(),
        credentials: credentials.as_ref().map(s3_credentials),
    }))
}

#[cfg(not(feature = "upload-s3"))]
fn s3_output_target(
    _bucket: &str,
    _prefix: &str,
    _stem: &str,
    _region: &Option<String>,
    _endpoint: &Option<String>,
    _credentials: &Option<GuiS3Credentials>,
) -> anyhow::Result<OutputTarget> {
    anyhow::bail!("S3 upload requires building the GUI with --features upload-s3")
}

#[cfg(feature = "upload-s3")]
fn test_s3_profile(settings: &GuiS3Settings) -> anyhow::Result<()> {
    img2omezarr::convert::upload::test_s3_connection(
        &settings.bucket,
        &settings.prefix,
        &settings.region,
        &settings.endpoint,
        &settings.credentials.as_ref().map(s3_credentials),
    )
}

#[cfg(feature = "upload-s3")]
fn s3_credentials(credentials: &GuiS3Credentials) -> img2omezarr::convert::config::S3Credentials {
    img2omezarr::convert::config::S3Credentials {
        access_key_id: credentials.access_key_id.clone(),
        secret_access_key: credentials.secret_access_key.clone(),
        session_token: None,
    }
}

#[cfg(not(feature = "upload-s3"))]
fn test_s3_profile(_settings: &GuiS3Settings) -> anyhow::Result<()> {
    anyhow::bail!("S3 upload requires building the GUI with --features upload-s3")
}

fn optional_string(value: SharedString) -> Option<String> {
    optional_str(value.as_str())
}

fn optional_str(value: &str) -> Option<String> {
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
        max_workers: 1,
        ..ConversionSettings::default()
    }
}

fn refresh_files(window: &slint::Weak<AppWindow>, state: &Rc<RefCell<State>>) {
    let Some(window) = window.upgrade() else {
        return;
    };
    let files = state.borrow().files.clone();
    let rows = files
        .iter()
        .map(|path| QueueFile {
            path: SharedString::from(path.display().to_string()),
            output: SharedString::from(preview_output_for_input(&window, state, path)),
            status: SharedString::from("queued"),
        })
        .collect::<Vec<_>>();
    window.set_files(ModelRc::new(VecModel::from(rows)));
}

fn preview_output_for_input(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    input: &Path,
) -> String {
    let output_dir = state.borrow().output_dir.clone();
    preview_output_for_target(
        input,
        output_dir.as_deref(),
        window.get_upload_mode().as_str(),
        window.get_s3_bucket().as_str(),
        window.get_s3_prefix().as_str(),
    )
}

fn preview_output_for_target(
    input: &Path,
    output_dir: Option<&Path>,
    upload_mode: &str,
    s3_bucket: &str,
    s3_prefix: &str,
) -> String {
    let stem = output_stem(input);
    if upload_mode == "s3" {
        let bucket = s3_bucket.trim();
        let prefix = s3_prefix.trim().trim_matches('/');
        if bucket.is_empty() || prefix.is_empty() {
            return "configure S3 bucket/prefix".to_string();
        }
        return format!("s3://{bucket}/{prefix}/{stem}.ome.zarr");
    }

    match output_dir {
        Some(output_dir) => output_dir
            .join(format!("{stem}.ome.zarr"))
            .display()
            .to_string(),
        None => "choose output folder".to_string(),
    }
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

fn show_error(window: &AppWindow, message: &str) {
    append_log(window, message);
    window.set_error_message(SharedString::from(message));
    window.set_error_visible(true);
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
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

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

    #[test]
    fn preview_output_shows_local_and_s3_targets() {
        assert_eq!(
            preview_output_for_target(
                Path::new("/inputs/cell.ome.tif"),
                Some(Path::new("/out")),
                "local",
                "",
                ""
            ),
            "/out/cell.ome.ome.zarr"
        );
        assert_eq!(
            preview_output_for_target(
                Path::new("/inputs/cell.ome.tif"),
                None,
                "s3",
                "bucket",
                "/prefix/"
            ),
            "s3://bucket/prefix/cell.ome.ome.zarr"
        );
        assert_eq!(
            preview_output_for_target(Path::new("/inputs/cell.ome.tif"), None, "local", "", ""),
            "choose output folder"
        );
        assert_eq!(
            preview_output_for_target(Path::new("/inputs/cell.ome.tif"), None, "s3", "bucket", ""),
            "configure S3 bucket/prefix"
        );
    }

    #[test]
    fn duplicate_output_targets_are_rejected() {
        let jobs = vec![
            ConvertJob {
                input: PathBuf::from("/a/image.tif"),
                output: OutputTarget::Local(PathBuf::from("/out/image.ome.zarr")),
            },
            ConvertJob {
                input: PathBuf::from("/b/image.nd2"),
                output: OutputTarget::Local(PathBuf::from("/out/image.ome.zarr")),
            },
        ];

        let err = validate_unique_output_targets(&jobs).unwrap_err();
        assert!(err
            .to_string()
            .contains("same output target: /a/image.tif and /b/image.nd2"));
    }

    #[test]
    fn s3_settings_use_saved_profile_credentials_when_fields_are_empty() {
        let _guard = env_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("IMG2OMEZARR_GUI_CONFIG_DIR", temp.path());

        let profile = config::profile_for_fields(
            "lab".to_string(),
            "bucket".to_string(),
            "prefix".to_string(),
            Some("us-east-1".to_string()),
            Some("http://127.0.0.1:9000".to_string()),
        );
        config::save(&config::AppConfig {
            default_profile: Some("lab".to_string()),
            s3_profiles: vec![profile.clone()],
            ..config::AppConfig::default()
        })
        .expect("save app config");
        config::save_profile_secrets(&profile, Some("saved-access"), Some("saved-secret"))
            .expect("save profile secrets");

        let settings = s3_settings_from_fields(
            "lab",
            "bucket",
            "prefix",
            "us-east-1",
            "http://127.0.0.1:9000",
            "",
            "",
        )
        .expect("s3 settings from saved credentials");
        let credentials = settings.credentials.expect("saved credentials");
        assert_eq!(credentials.access_key_id, "saved-access");
        assert_eq!(credentials.secret_access_key, "saved-secret");
        assert_eq!(settings.region.as_deref(), Some("us-east-1"));
        assert_eq!(settings.endpoint.as_deref(), Some("http://127.0.0.1:9000"));

        std::env::remove_var("IMG2OMEZARR_GUI_CONFIG_DIR");
    }

    #[test]
    fn s3_settings_reject_incomplete_saved_profile_credentials() {
        let _guard = env_lock();
        let temp = tempfile::tempdir().expect("tempdir");
        std::env::set_var("IMG2OMEZARR_GUI_CONFIG_DIR", temp.path());

        let profile = config::profile_for_fields(
            "lab".to_string(),
            "bucket".to_string(),
            "prefix".to_string(),
            None,
            None,
        );
        config::save(&config::AppConfig {
            default_profile: Some("lab".to_string()),
            s3_profiles: vec![profile.clone()],
            ..config::AppConfig::default()
        })
        .expect("save app config");
        config::save_profile_secrets(&profile, Some("saved-access"), None)
            .expect("save incomplete profile secrets");

        let err = s3_settings_from_fields("lab", "bucket", "prefix", "", "", "", "")
            .expect_err("incomplete saved credentials should fail");
        assert!(err
            .to_string()
            .contains("saved S3 profile credentials are incomplete"));

        std::env::remove_var("IMG2OMEZARR_GUI_CONFIG_DIR");
    }
}
