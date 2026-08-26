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
use bioformats_rs::{ImageMetadata, MetadataValue, OmeMetadata};

#[cfg(feature = "core-bioformats")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HcsImagePath {
    pub series: usize,
    pub well_path: String,
    pub field_path: String,
    pub image_path: String,
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
) -> Value {
    let image = ome.and_then(|ome| ome.images.get(series_index));
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
            if matches!(axis.name.kind(), AxisKind::Space) {
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
                    "z" => physical_size(image, meta, PhysicalAxis::Z),
                    _ => 1.0,
                })
                .collect::<Vec<_>>();
            json!({
                "path": level.index.to_string(),
                "coordinateTransformations": [{
                    "type": "scale",
                    "scale": scale,
                }],
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
        ome_attrs["omero"] = omero_metadata(series_index, meta, ome);
    }

    json!({ "ome": ome_attrs })
}

#[cfg(feature = "core-bioformats")]
#[derive(Debug, Clone, Copy)]
enum PhysicalAxis {
    X,
    Y,
    Z,
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
fn omero_metadata(series_index: usize, meta: &ImageMetadata, ome: Option<&OmeMetadata>) -> Value {
    let channel_count = meta.size_c.max(1) as usize;
    let color_render = channel_count > 1 && channel_count < 8;
    let default_range = pixel_range(meta.pixel_type).unwrap_or((0.0, 1.0));
    let image = ome.and_then(|ome| ome.images.get(series_index));
    let channels = (0..channel_count)
        .map(|c| {
            let channel = image.and_then(|image| image.channels.get(c));
            let label = channel
                .and_then(|channel| channel.name.clone())
                .unwrap_or_else(|| format!("Channel {c}"));
            let color = default_channel_color(c);
            json!({
                "active": c < 3,
                "coefficient": 1,
                "color": color,
                "family": "linear",
                "inverted": false,
                "label": label,
                "window": {
                    "start": default_range.0,
                    "end": default_range.1,
                    "min": default_range.0,
                    "max": default_range.1,
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
