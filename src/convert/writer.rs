use std::fs;
use std::path::Path;
use std::sync::Arc;

use ndarray::{ArrayD, IxDyn};
use zarrs::array::codec::bytes_to_bytes::blosc::{
    BloscCodec, BloscCompressionLevel, BloscCompressor, BloscShuffleMode,
};
use zarrs::array::codec::bytes_to_bytes::zstd::ZstdCodec;
use zarrs::array::{
    Array as ZarrArray, ArrayBuilder, ArraySubset, BytesToBytesCodecTraits, FillValue,
};
use zarrs::filesystem::FilesystemStore;
use zarrs::group::{Group, GroupBuilder};
use zarrs::metadata::v2::{ArrayMetadataV2, GroupMetadataV2};
use zarrs::metadata::{ArrayMetadata, ChunkKeySeparator, FillValueMetadata, GroupMetadata};

use crate::convert::axes::{chunks, shape, tczyx_axes};
use crate::convert::config::{
    CompressionCodec, ConversionSettings, Downsampling, NgffVersion, ResolutionPolicy,
};
use crate::convert::downsample::downsample_bytes;
use crate::convert::dtype::{bytes_per_sample, zarr_data_type, zarr_v2_data_type};
use crate::convert::metadata::{
    hcs_image_paths, ome_group_attributes, root_attributes, root_attributes_with_hcs_plate,
    series_attributes, well_attributes, HcsImagePath,
};
use crate::convert::planner::{
    plan_resolutions, selected_series, source_resolution_level, ResolutionLevel, ResolutionSource,
};
use crate::convert::progress::ProgressSink;
use crate::convert::reader::{plane_index, BioformatsImageSource};

use bioformats_rs::{ImageMetadata, PixelType};

pub fn write_omezarr<P>(
    source: &mut BioformatsImageSource,
    output: &Path,
    settings: &ConversionSettings,
    progress: &P,
) -> anyhow::Result<Vec<usize>>
where
    P: ProgressSink,
{
    validate_version_settings(settings)?;
    prepare_output(output, settings.overwrite)?;
    let store = Arc::new(FilesystemStore::new(output)?);
    let selected = selected_series(&settings.series, source.series_count())?;
    let collection_ome = source.ome_metadata();
    let hcs_paths = collection_ome
        .as_ref()
        .map(|ome| hcs_image_paths(ome, &selected))
        .unwrap_or_default();
    let root_attrs = if hcs_paths.is_empty() {
        root_attributes(settings.ngff_version)
    } else {
        root_attributes_with_hcs_plate(
            settings.ngff_version,
            collection_ome
                .as_ref()
                .expect("HCS paths require OME metadata"),
            &hcs_paths,
        )
    };
    write_group(&store, "/", root_attrs, settings.ngff_version)?;

    let series_paths = selected
        .iter()
        .map(|series| image_path_for_series(*series, &hcs_paths))
        .collect::<Vec<_>>();
    write_group(
        &store,
        "/OME",
        ome_group_attributes(settings.ngff_version, &series_paths),
        settings.ngff_version,
    )?;

    let mut written = Vec::with_capacity(selected.len());
    let mut wrote_ome_xml = false;
    for series in selected {
        source.set_series(series)?;
        source.set_resolution(0)?;
        let meta = source.metadata();
        let ome = source.ome_metadata();
        if settings.write_ome_xml && !wrote_ome_xml {
            if let Some(ome) = ome.as_ref() {
                write_ome_xml(output, &meta, ome)?;
                wrote_ome_xml = true;
            }
        }
        let levels = resolution_levels(source, &meta, settings)?;
        write_series(
            source,
            output,
            &store,
            settings,
            series,
            &image_path_for_series(series, &hcs_paths),
            &hcs_paths,
            &meta,
            ome.as_ref(),
            &levels,
            progress,
        )?;
        written.push(series);
    }
    Ok(written)
}

fn validate_version_settings(settings: &ConversionSettings) -> anyhow::Result<()> {
    if settings.ngff_version == NgffVersion::V04
        && settings.compression.codec == CompressionCodec::Zstd
    {
        anyhow::bail!("OME-Zarr 0.4 output supports --compression blosc or none; use 0.5 for zstd");
    }
    Ok(())
}

fn image_path_for_series(series: usize, hcs_paths: &[HcsImagePath]) -> String {
    hcs_paths
        .iter()
        .find(|path| path.series == series)
        .map(|path| path.image_path.clone())
        .unwrap_or_else(|| series.to_string())
}

fn resolution_levels(
    source: &mut BioformatsImageSource,
    base_meta: &ImageMetadata,
    settings: &ConversionSettings,
) -> anyhow::Result<Vec<ResolutionLevel>> {
    let resolution_count = source.resolution_count();
    if matches!(
        settings.resolution_policy,
        ResolutionPolicy::ExistingOrTargetMinSize(_)
    ) && resolution_count > 1
    {
        let mut levels = Vec::with_capacity(resolution_count);
        for source_resolution in 0..resolution_count {
            source.set_resolution(source_resolution)?;
            let level_meta = source.metadata();
            levels.push(source_resolution_level(
                source_resolution,
                source_resolution,
                base_meta.size_x as usize,
                base_meta.size_y as usize,
                level_meta.size_x as usize,
                level_meta.size_y as usize,
            ));
        }
        source.set_resolution(0)?;
        return Ok(levels);
    }

    Ok(plan_resolutions(
        base_meta.size_x as usize,
        base_meta.size_y as usize,
        settings.resolution_policy,
        resolution_count,
    ))
}

fn prepare_output(output: &Path, overwrite: bool) -> anyhow::Result<()> {
    if output.exists() {
        if overwrite {
            if output.is_dir() {
                fs::remove_dir_all(output)?;
            } else {
                fs::remove_file(output)?;
            }
        } else {
            anyhow::bail!("output already exists: {}", output.display());
        }
    }
    fs::create_dir_all(output)?;
    Ok(())
}

fn write_ome_xml(
    output: &Path,
    meta: &ImageMetadata,
    ome: &bioformats_rs::OmeMetadata,
) -> anyhow::Result<()> {
    let ome_dir = output.join("OME");
    fs::create_dir_all(&ome_dir)?;
    fs::write(ome_dir.join("METADATA.ome.xml"), ome.to_ome_xml(meta))?;
    Ok(())
}

fn write_series<P>(
    source: &mut BioformatsImageSource,
    output: &Path,
    store: &Arc<FilesystemStore>,
    settings: &ConversionSettings,
    series: usize,
    image_path: &str,
    hcs_paths: &[HcsImagePath],
    meta: &ImageMetadata,
    ome: Option<&bioformats_rs::OmeMetadata>,
    levels: &[ResolutionLevel],
    progress: &P,
) -> anyhow::Result<()>
where
    P: ProgressSink,
{
    let total_chunks = levels
        .iter()
        .map(|level| chunk_count(meta, level, settings))
        .sum();
    progress.series_started(series, levels.len(), total_chunks);

    let base_axes = tczyx_axes(
        meta.size_t as usize,
        meta.size_c as usize,
        meta.size_z as usize,
        meta.size_y as usize,
        meta.size_x as usize,
        settings.tile_height,
        settings.tile_width,
        settings.chunk_depth,
    );
    let attrs = series_attributes(settings, series, meta, ome, &base_axes, levels);
    write_parent_hcs_groups(store, settings.ngff_version, image_path, hcs_paths)?;
    write_group(
        store,
        &format!("/{image_path}"),
        attrs,
        settings.ngff_version,
    )?;

    let mut done = 0usize;
    for level in levels {
        let axes = tczyx_axes(
            meta.size_t as usize,
            meta.size_c as usize,
            meta.size_z as usize,
            level.size_y,
            level.size_x,
            settings.tile_height,
            settings.tile_width,
            settings.chunk_depth,
        );
        let node = format!("/{image_path}/{}", level.index);
        create_array(
            store,
            &node,
            &shape(&axes),
            &chunks(&axes),
            &axis_names(&axes),
            meta.pixel_type,
            meta.is_little_endian,
            settings,
        )?;
        write_level(
            source,
            output,
            settings,
            image_path,
            series,
            meta,
            level,
            &mut done,
            total_chunks,
            progress,
        )?;
    }
    Ok(())
}

fn write_level<P>(
    source: &mut BioformatsImageSource,
    output: &Path,
    settings: &ConversionSettings,
    image_path: &str,
    series: usize,
    meta: &ImageMetadata,
    level: &ResolutionLevel,
    done: &mut usize,
    total_chunks: usize,
    progress: &P,
) -> anyhow::Result<()>
where
    P: ProgressSink,
{
    match level.source {
        ResolutionSource::Generated => source.set_resolution(0)?,
        ResolutionSource::Source(source_resolution) => source.set_resolution(source_resolution)?,
    }

    let size_z = meta.size_z.max(1) as usize;
    let size_c = meta.size_c.max(1) as usize;
    let size_t = meta.size_t.max(1) as usize;
    let bps = bytes_per_sample(meta.pixel_type);
    let node_path = output.join(image_path).join(level.index.to_string());

    for t in 0..size_t {
        for c in 0..size_c {
            for z0 in (0..size_z).step_by(settings.chunk_depth.max(1)) {
                let depth = settings.chunk_depth.max(1).min(size_z - z0);
                for y in (0..level.size_y).step_by(settings.tile_height.max(1)) {
                    let height = settings.tile_height.max(1).min(level.size_y - y);
                    for x in (0..level.size_x).step_by(settings.tile_width.max(1)) {
                        let width = settings.tile_width.max(1).min(level.size_x - x);
                        let bytes = read_chunk_bytes(
                            source,
                            meta,
                            settings.downsampling,
                            level,
                            x,
                            y,
                            z0,
                            width,
                            height,
                            depth,
                            c,
                            t,
                            bps,
                        )?;
                        let origin = [t as u64, c as u64, z0 as u64, y as u64, x as u64];
                        let region_shape = [1usize, 1, depth, height, width];
                        write_bytes_region(
                            &node_path,
                            &origin,
                            &region_shape,
                            meta.pixel_type,
                            meta.is_little_endian,
                            &bytes,
                        )?;
                        *done += 1;
                        progress.chunk_finished(series, level.index, *done, total_chunks);
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn read_chunk_bytes(
    source: &mut BioformatsImageSource,
    meta: &ImageMetadata,
    downsampling: Downsampling,
    level: &ResolutionLevel,
    x: usize,
    y: usize,
    z0: usize,
    width: usize,
    height: usize,
    depth: usize,
    c: usize,
    t: usize,
    bps: usize,
) -> anyhow::Result<Vec<u8>> {
    let scale = level.scale_x.max(1) as usize;
    let mut out = Vec::with_capacity(width * height * depth * bps);
    for dz in 0..depth {
        let z = z0 + dz;
        let plane = plane_index(
            meta.dimension_order,
            z as u32,
            c as u32,
            t as u32,
            meta.size_z.max(1),
            meta.size_c.max(1),
            meta.size_t.max(1),
        );
        if matches!(level.source, ResolutionSource::Source(_)) || level.index == 0 {
            let plane_bytes =
                source.open_region(plane, x as u32, y as u32, width as u32, height as u32)?;
            out.extend_from_slice(&plane_bytes);
        } else {
            let sx = x * scale;
            let sy = y * scale;
            let sw = (width * scale).min(meta.size_x as usize - sx);
            let sh = (height * scale).min(meta.size_y as usize - sy);
            let plane_bytes =
                source.open_region(plane, sx as u32, sy as u32, sw as u32, sh as u32)?;
            out.extend(downsample_bytes(
                &plane_bytes,
                sw,
                sh,
                meta.pixel_type,
                meta.is_little_endian,
                scale,
                downsampling,
            ));
        }
    }
    Ok(out)
}

fn write_group(
    store: &Arc<FilesystemStore>,
    node: &str,
    attributes: serde_json::Value,
    version: NgffVersion,
) -> anyhow::Result<()> {
    let attrs = group_attributes_for_version(attributes, version)?
        .as_object()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("group attributes must be a JSON object"))?;
    let group = match version {
        NgffVersion::V05 => {
            let mut builder = GroupBuilder::new();
            builder.attributes(attrs);
            builder.build(store.clone(), node)?
        }
        NgffVersion::V04 => Group::new_with_metadata(
            store.clone(),
            node,
            GroupMetadata::V2(GroupMetadataV2::new().with_attributes(attrs)),
        )?,
    };
    group.store_metadata()?;
    Ok(())
}

fn group_attributes_for_version(
    attributes: serde_json::Value,
    version: NgffVersion,
) -> anyhow::Result<serde_json::Value> {
    match version {
        NgffVersion::V05 => Ok(attributes),
        NgffVersion::V04 => Ok(legacy_ngff_attributes(attributes)),
    }
}

fn legacy_ngff_attributes(attributes: serde_json::Value) -> serde_json::Value {
    let Some(mut ome) = attributes
        .get("ome")
        .and_then(|ome| ome.as_object())
        .cloned()
    else {
        return attributes;
    };
    ome.remove("version");
    if let Some(plate) = ome.get_mut("plate").and_then(|plate| plate.as_object_mut()) {
        plate.remove("version");
    }
    if let Some(well) = ome.get_mut("well").and_then(|well| well.as_object_mut()) {
        well.remove("version");
    }
    if let Some(multiscales) = ome
        .get_mut("multiscales")
        .and_then(|value| value.as_array_mut())
    {
        for multiscale in multiscales {
            if let Some(multiscale) = multiscale.as_object_mut() {
                multiscale
                    .entry("version")
                    .or_insert_with(|| serde_json::Value::String("0.4".to_string()));
            }
        }
    }
    serde_json::Value::Object(ome)
}

fn write_parent_hcs_groups(
    store: &Arc<FilesystemStore>,
    version: NgffVersion,
    image_path: &str,
    hcs_paths: &[HcsImagePath],
) -> anyhow::Result<()> {
    let mut parts = image_path.split('/');
    let (Some(row), Some(column), Some(_field), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Ok(());
    };

    write_group(
        store,
        &format!("/{row}"),
        serde_json::json!({ "ome": { "version": version.as_str() } }),
        version,
    )?;

    let well_path = format!("{row}/{column}");
    let images = hcs_paths
        .iter()
        .filter(|path| path.well_path == well_path)
        .cloned()
        .collect::<Vec<_>>();
    write_group(
        store,
        &format!("/{well_path}"),
        well_attributes(version, &images),
        version,
    )?;
    Ok(())
}

fn create_array(
    store: &Arc<FilesystemStore>,
    node: &str,
    shape: &[usize],
    chunks: &[usize],
    dimension_names: &[String],
    pixel_type: PixelType,
    little_endian: bool,
    settings: &ConversionSettings,
) -> anyhow::Result<()> {
    let shape_u64: Vec<u64> = shape.iter().map(|&dim| dim as u64).collect();
    let chunks_u64: Vec<u64> = chunks.iter().map(|&dim| dim as u64).collect();
    let array = match settings.ngff_version {
        NgffVersion::V05 => create_v3_array(
            store,
            node,
            shape_u64,
            chunks_u64,
            dimension_names,
            pixel_type,
            settings,
        )?,
        NgffVersion::V04 => create_v2_array(
            store,
            node,
            shape_u64,
            chunks_u64,
            pixel_type,
            little_endian,
            settings,
        )?,
    };
    array.store_metadata()?;
    Ok(())
}

fn create_v3_array(
    store: &Arc<FilesystemStore>,
    node: &str,
    shape: Vec<u64>,
    chunks: Vec<u64>,
    dimension_names: &[String],
    pixel_type: PixelType,
    settings: &ConversionSettings,
) -> anyhow::Result<ZarrArray<FilesystemStore>> {
    let data_type = zarr_data_type(pixel_type);
    let mut builder =
        ArrayBuilder::new(shape, chunks.as_slice(), data_type, fill_value(pixel_type));
    builder.dimension_names(Some(dimension_names.iter().map(String::as_str)));
    if settings.compression.codec == CompressionCodec::Zstd {
        let level = settings.compression.level.unwrap_or(3);
        builder.bytes_to_bytes_codecs(vec![
            Arc::new(ZstdCodec::new(level, false)) as Arc<dyn BytesToBytesCodecTraits>
        ]);
    } else if settings.compression.codec == CompressionCodec::Blosc {
        builder.bytes_to_bytes_codecs(vec![Arc::new(blosc_codec(pixel_type, settings)?)]);
    }
    Ok(builder.build(store.clone(), node)?)
}

fn create_v2_array(
    store: &Arc<FilesystemStore>,
    node: &str,
    shape: Vec<u64>,
    chunks: Vec<u64>,
    pixel_type: PixelType,
    little_endian: bool,
    settings: &ConversionSettings,
) -> anyhow::Result<ZarrArray<FilesystemStore>> {
    let chunks = chunks
        .into_iter()
        .map(|chunk| {
            std::num::NonZeroU64::new(chunk)
                .ok_or_else(|| anyhow::anyhow!("Zarr chunk dimensions must be non-zero"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let metadata = ArrayMetadataV2::new(
        shape,
        chunks,
        zarr_v2_data_type(pixel_type, little_endian).into(),
        fill_value_v2(pixel_type),
        blosc_v2_compressor(pixel_type, settings)?,
        None,
    )
    .with_dimension_separator(ChunkKeySeparator::Slash);
    Ok(ZarrArray::new_with_metadata(
        store.clone(),
        node,
        ArrayMetadata::V2(metadata),
    )?)
}

fn blosc_codec(pixel_type: PixelType, settings: &ConversionSettings) -> anyhow::Result<BloscCodec> {
    Ok(BloscCodec::new(
        BloscCompressor::Zstd,
        blosc_level(settings)?,
        None,
        BloscShuffleMode::Shuffle,
        Some(bytes_per_sample(pixel_type).max(1)),
    )?)
}

fn blosc_v2_compressor(
    pixel_type: PixelType,
    settings: &ConversionSettings,
) -> anyhow::Result<Option<zarrs::metadata::v2::MetadataV2>> {
    if settings.compression.codec != CompressionCodec::Blosc {
        return Ok(None);
    }
    Ok(Some(serde_json::from_value(serde_json::json!({
        "id": "blosc",
        "cname": "zstd",
        "clevel": u8::from(blosc_level(settings)?),
        "shuffle": 1,
        "blocksize": 0,
        "typesize": bytes_per_sample(pixel_type).max(1),
    }))?))
}

fn blosc_level(settings: &ConversionSettings) -> anyhow::Result<BloscCompressionLevel> {
    let level = settings.compression.level.unwrap_or(5).clamp(0, 9) as u8;
    BloscCompressionLevel::try_from(level).map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn axis_names(axes: &[crate::convert::axes::Axis]) -> Vec<String> {
    axes.iter()
        .map(|axis| axis.name.as_str().to_string())
        .collect()
}

fn fill_value(pixel_type: PixelType) -> FillValue {
    match pixel_type {
        PixelType::Bit => FillValue::from(0u8),
        PixelType::Int8 => FillValue::from(0i8),
        PixelType::Uint8 => FillValue::from(0u8),
        PixelType::Int16 => FillValue::from(0i16),
        PixelType::Uint16 => FillValue::from(0u16),
        PixelType::Int32 => FillValue::from(0i32),
        PixelType::Uint32 => FillValue::from(0u32),
        PixelType::Float32 => FillValue::from(0f32),
        PixelType::Float64 => FillValue::from(0f64),
    }
}

fn fill_value_v2(pixel_type: PixelType) -> FillValueMetadata {
    match pixel_type {
        PixelType::Bit => FillValueMetadata::from(0u8),
        PixelType::Int8 => FillValueMetadata::from(0i8),
        PixelType::Uint8 => FillValueMetadata::from(0u8),
        PixelType::Int16 => FillValueMetadata::from(0i16),
        PixelType::Uint16 => FillValueMetadata::from(0u16),
        PixelType::Int32 => FillValueMetadata::from(0i32),
        PixelType::Uint32 => FillValueMetadata::from(0u32),
        PixelType::Float32 => FillValueMetadata::from(0f32),
        PixelType::Float64 => FillValueMetadata::from(0f64),
    }
}

fn write_bytes_region(
    path: &Path,
    origin: &[u64; 5],
    shape: &[usize; 5],
    pixel_type: PixelType,
    little_endian: bool,
    bytes: &[u8],
) -> anyhow::Result<()> {
    match pixel_type {
        PixelType::Bit => write_typed_region::<u8>(path, origin, shape, bytes_to_u8(bytes)),
        PixelType::Int8 => write_typed_region::<i8>(path, origin, shape, bytes_to_i8(bytes)),
        PixelType::Uint8 => write_typed_region::<u8>(path, origin, shape, bytes_to_u8(bytes)),
        PixelType::Int16 => {
            write_typed_region::<i16>(path, origin, shape, bytes_to_i16(bytes, little_endian))
        }
        PixelType::Uint16 => {
            write_typed_region::<u16>(path, origin, shape, bytes_to_u16(bytes, little_endian))
        }
        PixelType::Int32 => {
            write_typed_region::<i32>(path, origin, shape, bytes_to_i32(bytes, little_endian))
        }
        PixelType::Uint32 => {
            write_typed_region::<u32>(path, origin, shape, bytes_to_u32(bytes, little_endian))
        }
        PixelType::Float32 => {
            write_typed_region::<f32>(path, origin, shape, bytes_to_f32(bytes, little_endian))
        }
        PixelType::Float64 => {
            write_typed_region::<f64>(path, origin, shape, bytes_to_f64(bytes, little_endian))
        }
    }
}

fn write_typed_region<T>(
    path: &Path,
    origin: &[u64; 5],
    shape: &[usize; 5],
    elements: Vec<T>,
) -> anyhow::Result<()>
where
    T: Clone + zarrs::array::Element,
{
    let expected = shape.iter().product::<usize>();
    if elements.len() != expected {
        anyhow::bail!(
            "region has {} elements, expected {expected}",
            elements.len()
        );
    }
    let store = Arc::new(FilesystemStore::new(path)?);
    let array = ZarrArray::open(store, "/")?;
    let shape_u64: Vec<u64> = shape.iter().map(|&dim| dim as u64).collect();
    let subset = ArraySubset::new_with_start_shape(origin.to_vec(), shape_u64)?;
    let data = ArrayD::from_shape_vec(IxDyn(shape), elements)?;
    let values = data.iter().cloned().collect::<Vec<_>>();
    array.store_array_subset(&subset, values)?;
    Ok(())
}

fn chunk_count(
    meta: &ImageMetadata,
    level: &ResolutionLevel,
    settings: &ConversionSettings,
) -> usize {
    let z_chunks = div_ceil(meta.size_z.max(1) as usize, settings.chunk_depth.max(1));
    let y_chunks = div_ceil(level.size_y, settings.tile_height.max(1));
    let x_chunks = div_ceil(level.size_x, settings.tile_width.max(1));
    meta.size_t.max(1) as usize * meta.size_c.max(1) as usize * z_chunks * y_chunks * x_chunks
}

fn div_ceil(n: usize, d: usize) -> usize {
    n.div_ceil(d)
}

fn bytes_to_u8(bytes: &[u8]) -> Vec<u8> {
    bytes.to_vec()
}

fn bytes_to_i8(bytes: &[u8]) -> Vec<i8> {
    bytes.iter().map(|&b| b as i8).collect()
}

macro_rules! bytes_to_num {
    ($name:ident, $ty:ty, $width:expr) => {
        fn $name(bytes: &[u8], little_endian: bool) -> Vec<$ty> {
            bytes
                .chunks_exact($width)
                .map(|chunk| {
                    let chunk = chunk.try_into().expect("chunk width");
                    if little_endian {
                        <$ty>::from_le_bytes(chunk)
                    } else {
                        <$ty>::from_be_bytes(chunk)
                    }
                })
                .collect()
        }
    };
}

bytes_to_num!(bytes_to_i16, i16, 2);
bytes_to_num!(bytes_to_u16, u16, 2);
bytes_to_num!(bytes_to_i32, i32, 4);
bytes_to_num!(bytes_to_u32, u32, 4);
bytes_to_num!(bytes_to_f32, f32, 4);
bytes_to_num!(bytes_to_f64, f64, 8);
