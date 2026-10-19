use serde::Serialize;
use serde_json::{json, Value};

#[cfg(feature = "core-bioformats")]
use crate::convert::axes::{Axis, AxisKind};
#[cfg(feature = "core-bioformats")]
use crate::convert::config::ConversionSettings;
use crate::convert::config::NgffVersion;
#[cfg(feature = "core-bioformats")]
use crate::convert::dtype::pixel_range;
#[cfg(feature = "core-bioformats")]
use crate::convert::planner::ResolutionLevel;
#[cfg(feature = "core-bioformats")]
use crate::convert::positions::Placement;

#[cfg(feature = "core-bioformats")]
use bioformats_rs::{ImageMetadata, MetadataValue, OmeMetadata};

#[cfg(feature = "core-bioformats")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcsImagePath {
    pub series: usize,
    pub well_path: String,
    pub field_path: String,
    pub image_path: String,
}

/// Display limits written to `omero.channels[].window`.
///
/// `min` and `max` describe the observed data bounds. `start` and `end` are
/// the initial display window, which may be narrower for useful auto contrast.
#[cfg(feature = "core-bioformats")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelWindow {
    pub min: f64,
    pub max: f64,
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Serialize)]
struct OmeWrapper<T> {
    ome: T,
}

#[derive(Debug, Serialize)]
struct RootOme<'a> {
    version: &'a str,
    #[serde(rename = "bioformats2raw.layout")]
    bioformats2raw_layout: u8,
}

pub fn root_attributes(version: NgffVersion) -> Value {
    json!(OmeWrapper {
        ome: RootOme {
            version: version.as_str(),
            bioformats2raw_layout: 3,
        }
    })
}

pub fn ome_group_attributes(version: NgffVersion, series_paths: &[String]) -> Value {
    json!({
        "ome": {
            "version": version.as_str(),
            "series": series_paths,
        }
    })
}

#[cfg(feature = "core-bioformats")]
pub fn hcs_image_paths(ome: &OmeMetadata, selected_series: &[usize]) -> Vec<HcsImagePath> {
    let Some(plate) = ome.plates.first() else {
        return Vec::new();
    };
    let selected = selected_series
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    let mut paths = Vec::new();
    for well in &plate.wells {
        let well_path = format!("{}/{}", row_name(well.row), well.column + 1);
        for sample in &well.well_samples {
            let Some(series) = sample.image_ref else {
                continue;
            };
            if !selected.contains(&series) {
                continue;
            }
            let field_path = sample.index.to_string();
            paths.push(HcsImagePath {
                series,
                well_path: well_path.clone(),
                image_path: format!("{well_path}/{field_path}"),
                field_path,
            });
        }
    }
    paths.sort_by_key(|path| series_position(selected_series, path.series));
    paths
}

#[cfg(feature = "core-bioformats")]
pub fn root_attributes_with_hcs_plate(
    version: NgffVersion,
    ome: &OmeMetadata,
    paths: &[HcsImagePath],
) -> Value {
    let Some(plate) = ome.plates.first() else {
        return root_attributes(version);
    };
    let mut root = root_attributes(version);
    let Some(ome_attrs) = root["ome"].as_object_mut() else {
        return root;
    };

    let rows = (0..plate.rows.max(max_row(plate)))
        .map(|row| json!({ "name": row_name(row) }))
        .collect::<Vec<_>>();
    let columns = (0..plate.columns.max(max_column(plate)))
        .map(|column| json!({ "name": (column + 1).to_string() }))
        .collect::<Vec<_>>();
    let wells = unique_well_paths(paths)
        .into_iter()
        .map(|well_path| {
            let (row, column) = parse_well_path(&well_path);
            json!({
                "path": well_path,
                "rowIndex": row,
                "columnIndex": column,
            })
        })
        .collect::<Vec<_>>();
    let field_count = paths
        .iter()
        .filter_map(|path| path.field_path.parse::<u32>().ok())
        .max()
        .map(|max| max + 1)
        .unwrap_or(0);

    ome_attrs.insert(
        "plate".to_string(),
        json!({
            "version": version.as_str(),
            "name": plate.name.clone().unwrap_or_else(|| "Plate".to_string()),
            "rows": rows,
            "columns": columns,
            "wells": wells,
            "field_count": field_count,
            "acquisitions": [{
                "id": 0,
                "maximumfieldcount": field_count.max(1),
            }],
        }),
    );
    root
}

#[cfg(feature = "core-bioformats")]
pub fn well_attributes(version: NgffVersion, images: &[HcsImagePath]) -> Value {
    let mut seen = std::collections::BTreeSet::new();
    let image_values = images
        .iter()
        .filter(|image| seen.insert(image.field_path.clone()))
        .map(|image| {
            json!({
                "path": image.field_path,
                "acquisition": 0,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "ome": {
            "version": version.as_str(),
            "well": {
                "version": version.as_str(),
                "images": image_values,
            }
        }
    })
}

#[cfg(feature = "core-bioformats")]
pub fn series_attributes(
    settings: &ConversionSettings,
    series_index: usize,
    meta: &ImageMetadata,
    ome: Option<&OmeMetadata>,
    axes: &[Axis],
    levels: &[ResolutionLevel],
    placement: Option<&Placement>,
    channel_windows: Option<&[ChannelWindow]>,
) -> Value {
    let image = ome.and_then(|ome| ome.images.get(series_index));
    // the tile position: the level-0 translation. The other levels carry no
    // offset of their own relative to level 0, so they get the same one.
    let translation = placement
        .and_then(|placement| placement.translation)
        .map(|zyx| {
            axes.iter()
                .map(|axis| match axis.name.as_str() {
                    "z" => zyx[0],
                    "y" => zyx[1],
                    "x" => zyx[2],
                    _ => 0.0,
                })
                .collect::<Vec<_>>()
        });
    let calibrated = spatial_sizes_calibrated(image);
    let axes_json: Vec<Value> = axes
        .iter()
        .map(|axis| {
            let mut value = json!({
                "name": axis.name.as_str(),
                "type": match axis.name.kind() {
                    AxisKind::Time => "time",
                    AxisKind::Channel => "channel",
                    AxisKind::Space => "space",
                },
            });
            if calibrated && matches!(axis.name.kind(), AxisKind::Space) {
                value["unit"] = json!("micrometer");
            }
            value
        })
        .collect();

    let datasets: Vec<Value> = levels
        .iter()
        .map(|level| {
            let scale = axes
                .iter()
                .map(|axis| match axis.name.as_str() {
                    "x" => physical_size(image, meta, PhysicalAxis::X) * f64::from(level.scale_x),
                    "y" => physical_size(image, meta, PhysicalAxis::Y) * f64::from(level.scale_y),
                    "z" => physical_size(image, meta, PhysicalAxis::Z) * f64::from(level.scale_z),
                    _ => 1.0,
                })
                .collect::<Vec<_>>();
            let mut transforms = vec![json!({ "type": "scale", "scale": scale })];
            if let Some(translation) = &translation {
                transforms.push(json!({ "type": "translation", "translation": translation }));
            }
            json!({
                "path": level.index.to_string(),
                "coordinateTransformations": transforms,
            })
        })
        .collect();

    let name = image
        .and_then(|image| image.name.clone())
        .or_else(|| {
            meta.series_metadata
                .get("ImageName")
                .map(std::string::ToString::to_string)
        })
        .unwrap_or_else(|| format!("Series {series_index}"));

    let method = match settings.downsampling {
        crate::convert::config::Downsampling::Nearest => {
            "img2omezarr nearest-neighbor byte downsampling"
        }
        crate::convert::config::Downsampling::Average => "img2omezarr average downsampling",
    };

    let mut ome_attrs = json!({
        "version": settings.ngff_version.as_str(),
        "multiscales": [{
            "name": name,
            "axes": axes_json,
            "datasets": datasets,
            "metadata": {
                "method": method,
                "version": env!("CARGO_PKG_VERSION"),
            }
        }]
    });

    if settings.write_omero_metadata {
        ome_attrs["omero"] = omero_metadata(series_index, meta, ome, channel_windows);
    }

    let mut attrs = json!({ "ome": ome_attrs });
    if let Some(placement) = placement {
        attrs["img2omezarr"] = json!({ "positions": placement.provenance() });
    }
    attrs
}

#[cfg(feature = "core-bioformats")]
#[derive(Debug, Clone, Copy)]
enum PhysicalAxis {
    X,
    Y,
    Z,
}

/// Whether the spatial sizes are lengths. bioformats reports them in
/// micrometres, except for an uncalibrated axis, which keeps a non-length unit
/// such as `pixel` (e.g. an ImageJ `unit=pixel` stack). NGFF has no such unit,
/// and one uncalibrated axis makes the others relative too (Bio-Formats labels
/// ImageJ's z spacing µm whatever its unit), so all are then written unitless.
#[cfg(feature = "core-bioformats")]
pub(crate) fn spatial_sizes_calibrated(image: Option<&bioformats_rs::OmeImage>) -> bool {
    image.is_none_or(|image| {
        image.physical_size_x_unit.is_none()
            && image.physical_size_y_unit.is_none()
            && image.physical_size_z_unit.is_none()
    })
}

#[cfg(feature = "core-bioformats")]
pub(crate) fn physical_sizes_xyz(
    image: Option<&bioformats_rs::OmeImage>,
    meta: &ImageMetadata,
) -> [f64; 3] {
    [
        physical_size(image, meta, PhysicalAxis::X),
        physical_size(image, meta, PhysicalAxis::Y),
        physical_size(image, meta, PhysicalAxis::Z),
    ]
}

#[cfg(feature = "core-bioformats")]
fn physical_size(
    image: Option<&bioformats_rs::OmeImage>,
    meta: &ImageMetadata,
    axis: PhysicalAxis,
) -> f64 {
    let from_ome = match axis {
        PhysicalAxis::X => image.and_then(|image| image.physical_size_x),
        PhysicalAxis::Y => image.and_then(|image| image.physical_size_y),
        PhysicalAxis::Z => image.and_then(|image| image.physical_size_z),
    };
    from_ome
        .or_else(|| physical_size_from_series_metadata(meta, axis))
        .unwrap_or(1.0)
}

#[cfg(feature = "core-bioformats")]
fn physical_size_from_series_metadata(meta: &ImageMetadata, axis: PhysicalAxis) -> Option<f64> {
    let keys: &[&str] = match axis {
        PhysicalAxis::X => &[
            "PhysicalSizeX",
            "physicalSizeX",
            "physical_size_x",
            "PixelsPhysicalSizeX",
        ],
        PhysicalAxis::Y => &[
            "PhysicalSizeY",
            "physicalSizeY",
            "physical_size_y",
            "PixelsPhysicalSizeY",
        ],
        PhysicalAxis::Z => &[
            "PhysicalSizeZ",
            "physicalSizeZ",
            "physical_size_z",
            "PixelsPhysicalSizeZ",
        ],
    };
    keys.iter()
        .filter_map(|key| meta.series_metadata.get(*key))
        .find_map(metadata_value_as_f64)
        .filter(|value| value.is_finite() && *value > 0.0)
}

#[cfg(feature = "core-bioformats")]
fn metadata_value_as_f64(value: &MetadataValue) -> Option<f64> {
    match value {
        MetadataValue::Float(value) => Some(*value),
        MetadataValue::Int(value) => Some(*value as f64),
        MetadataValue::String(value) => value.parse().ok(),
        MetadataValue::Bool(_) | MetadataValue::Bytes(_) => None,
    }
}

#[cfg(feature = "core-bioformats")]
fn display_range(meta: &ImageMetadata) -> (f64, f64) {
    let storage_range = pixel_range(meta.pixel_type).unwrap_or((0.0, 1.0));
    let bits = u32::from(meta.bits_per_pixel);
    let storage_bits = u32::try_from(meta.pixel_type.bytes_per_sample() * 8).unwrap_or(u32::MAX);
    if bits == 0 || bits >= storage_bits || bits > 32 {
        return storage_range;
    }
    match meta.pixel_type {
        bioformats_rs::PixelType::Bit
        | bioformats_rs::PixelType::Uint8
        | bioformats_rs::PixelType::Uint16
        | bioformats_rs::PixelType::Uint32 => (0.0, ((1_u64 << bits) - 1) as f64),
        bioformats_rs::PixelType::Int8
        | bioformats_rs::PixelType::Int16
        | bioformats_rs::PixelType::Int32 => {
            let magnitude = 1_i64 << (bits - 1);
            (-magnitude as f64, (magnitude - 1) as f64)
        }
        bioformats_rs::PixelType::Float32 | bioformats_rs::PixelType::Float64 => storage_range,
    }
}

#[cfg(feature = "core-bioformats")]
fn omero_metadata(
    series_index: usize,
    meta: &ImageMetadata,
    ome: Option<&OmeMetadata>,
    channel_windows: Option<&[ChannelWindow]>,
) -> Value {
    let channel_count = meta.size_c.max(1) as usize;
    let color_render = channel_count > 1 && channel_count < 8;
    // Camera data is commonly stored in uint16 containers with only 10, 12, or 14 meaningful
    // bits. Bio-Formats exposes that distinction as bits_per_pixel; using the container maximum
    // makes the initial image nearly black and gives contrast sliders a mostly useless range.
    let default_range = display_range(meta);
    let image = ome.and_then(|ome| ome.images.get(series_index));
    let channels = (0..channel_count)
        .map(|c| {
            let channel = image.and_then(|image| image.channels.get(c));
            let label = channel
                .and_then(|channel| channel.name.clone())
                .unwrap_or_else(|| format!("Channel {c}"));
            let color = default_channel_color(c);
            let window = channel_windows
                .and_then(|windows| windows.get(c))
                .copied()
                .unwrap_or(ChannelWindow {
                    min: default_range.0,
                    max: default_range.1,
                    start: default_range.0,
                    end: default_range.1,
                });
            json!({
                "active": c < 3,
                "coefficient": 1,
                "color": color,
                "family": "linear",
                "inverted": false,
                "label": label,
                "window": {
                    "start": window.start,
                    "end": window.end,
                    "min": window.min,
                    "max": window.max,
                },
            })
        })
        .collect::<Vec<_>>();
    json!({
        "rdefs": {
            "defaultT": 0,
            "defaultZ": meta.size_z.max(1) / 2,
            "model": if color_render { "color" } else { "greyscale" },
        },
        "channels": channels,
    })
}

#[cfg(feature = "core-bioformats")]
fn default_channel_color(index: usize) -> &'static str {
    const COLORS: [&str; 8] = [
        "FFFFFF", "FF0000", "00FF00", "0000FF", "FFFF00", "FF00FF", "00FFFF", "808080",
    ];
    COLORS[index % COLORS.len()]
}

#[cfg(feature = "core-bioformats")]
fn series_position(selected_series: &[usize], series: usize) -> usize {
    selected_series
        .iter()
        .position(|selected| *selected == series)
        .unwrap_or(usize::MAX)
}

#[cfg(feature = "core-bioformats")]
fn row_name(row: u32) -> String {
    let mut row = row + 1;
    let mut chars = Vec::new();
    while row > 0 {
        row -= 1;
        chars.push((b'A' + (row % 26) as u8) as char);
        row /= 26;
    }
    chars.iter().rev().collect()
}

#[cfg(feature = "core-bioformats")]
fn max_row(plate: &bioformats_rs::OmePlate) -> u32 {
    plate
        .wells
        .iter()
        .map(|well| well.row + 1)
        .max()
        .unwrap_or(0)
}

#[cfg(feature = "core-bioformats")]
fn max_column(plate: &bioformats_rs::OmePlate) -> u32 {
    plate
        .wells
        .iter()
        .map(|well| well.column + 1)
        .max()
        .unwrap_or(0)
}

#[cfg(feature = "core-bioformats")]
fn unique_well_paths(paths: &[HcsImagePath]) -> Vec<String> {
    paths
        .iter()
        .map(|path| path.well_path.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(feature = "core-bioformats")]
fn parse_well_path(path: &str) -> (u32, u32) {
    let Some((row, column)) = path.split_once('/') else {
        return (0, 0);
    };
    let row_index = row.chars().fold(0u32, |acc, ch| {
        acc * 26 + (ch.to_ascii_uppercase() as u32 - 'A' as u32 + 1)
    });
    let column_index = column.parse::<u32>().unwrap_or(1);
    (row_index.saturating_sub(1), column_index.saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_metadata_uses_ngff_05_by_default() {
        let attrs = root_attributes(NgffVersion::V05);
        assert_eq!(attrs["ome"]["version"], "0.5");
        assert_eq!(attrs["ome"]["bioformats2raw.layout"], 3);
    }

    #[cfg(feature = "core-bioformats")]
    #[test]
    fn uncalibrated_sizes_are_written_without_a_unit() {
        use crate::convert::axes::AxisName;
        use crate::convert::planner::{ResolutionLevel, ResolutionSource};

        let axes: Vec<Axis> = [AxisName::Z, AxisName::Y, AxisName::X]
            .into_iter()
            .map(|name| Axis {
                name,
                length: 4,
                chunk: 4,
            })
            .collect();
        let levels = [ResolutionLevel {
            index: 0,
            scale_z: 1,
            scale_x: 1,
            scale_y: 1,
            size_z: 4,
            size_x: 4,
            size_y: 4,
            source: ResolutionSource::Generated,
        }];
        let meta = ImageMetadata::default();
        // an ImageJ `unit=pixel` stack, as bioformats reports it
        let mut image = bioformats_rs::OmeImage {
            physical_size_x: Some(2.0),
            physical_size_y: Some(2.0),
            physical_size_z: Some(2.0),
            physical_size_x_unit: Some("pixel".into()),
            physical_size_y_unit: Some("pixel".into()),
            ..Default::default()
        };
        let attrs = |image: &bioformats_rs::OmeImage| {
            let ome = OmeMetadata {
                images: vec![image.clone()],
                ..Default::default()
            };
            series_attributes(
                &ConversionSettings::default(),
                0,
                &meta,
                Some(&ome),
                &axes,
                &levels,
                None,
                None,
            )
        };

        let uncalibrated = attrs(&image);
        let multiscale = &uncalibrated["ome"]["multiscales"][0];
        for axis in multiscale["axes"].as_array().unwrap() {
            assert!(axis.get("unit").is_none(), "{axis}");
        }
        assert_eq!(
            multiscale["datasets"][0]["coordinateTransformations"][0]["scale"],
            json!([2.0, 2.0, 2.0])
        );

        image.physical_size_x_unit = None;
        image.physical_size_y_unit = None;
        let calibrated = attrs(&image);
        assert_eq!(
            calibrated["ome"]["multiscales"][0]["axes"][2]["unit"],
            "micrometer"
        );
    }

    #[cfg(feature = "core-bioformats")]
    #[test]
    fn omero_window_uses_declared_significant_bits() {
        let meta = ImageMetadata {
            pixel_type: bioformats_rs::PixelType::Uint16,
            bits_per_pixel: 12,
            size_c: 2,
            ..Default::default()
        };
        let omero = omero_metadata(0, &meta, None, None);
        assert_eq!(omero["channels"][0]["window"]["start"], 0.0);
        assert_eq!(omero["channels"][0]["window"]["end"], 4095.0);
        assert_eq!(omero["channels"][1]["window"]["max"], 4095.0);
    }

    #[cfg(feature = "core-bioformats")]
    #[test]
    fn hcs_well_paths_follow_plate_row_column_field_layout() {
        let mut ome = OmeMetadata::default();
        ome.plates.push(bioformats_rs::OmePlate {
            name: Some("Plate A".to_string()),
            rows: 1,
            columns: 1,
            wells: vec![bioformats_rs::OmeWell {
                row: 0,
                column: 0,
                well_samples: vec![bioformats_rs::OmeWellSample {
                    index: 2,
                    image_ref: Some(4),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        });

        let paths = hcs_image_paths(&ome, &[4]);
        assert_eq!(paths[0].well_path, "A/1");
        assert_eq!(paths[0].field_path, "2");
        assert_eq!(paths[0].image_path, "A/1/2");

        let attrs = root_attributes_with_hcs_plate(NgffVersion::V05, &ome, &paths);
        assert_eq!(attrs["ome"]["plate"]["wells"][0]["path"], "A/1");
    }
}
