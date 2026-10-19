//! The standard OME source: `Plane.PositionX/Y/Z` and `StageLabel`, as
//! bioformats-rs returns them.
//!
//! The plane is the image's first one (`TheZ=0 TheC=0 TheT=0`, else the
//! first listed), whose z is the first voxel's. An axis the plane lacks is
//! taken from the `StageLabel`. bioformats-rs does not report the positions'
//! units (`PositionXUnit`); Bio-Formats' readers store them in micrometres,
//! so micrometres are assumed, and the provenance says so.

use std::path::Path;

use bioformats_rs::{ImageMetadata, OmeImage, OmeMetadata};

use super::lavision::{self, LaVisionSource};
use super::{file_name, StageReading, Tile};
use crate::convert::metadata::{physical_sizes_xyz, spatial_sizes_calibrated};
use crate::convert::reader::BioformatsImageSource;

/// A tile per selected series of an open input, with its stage reading.
pub fn tiles(
    source: &mut BioformatsImageSource,
    input: &Path,
    selected: &[usize],
) -> anyhow::Result<Vec<Tile>> {
    let ome = source.ome_metadata();
    let mut tiles = Vec::with_capacity(selected.len());
    for &series in selected {
        source.set_series(series)?;
        source.set_resolution(0)?;
        let meta = source.metadata();
        tiles.push(tile(input, series, &meta, ome.as_ref()));
    }
    Ok(tiles)
}

pub fn tile(input: &Path, series: usize, meta: &ImageMetadata, ome: Option<&OmeMetadata>) -> Tile {
    let image = ome.and_then(|ome| ome.images.get(series));
    let [x, y, z] = physical_sizes_xyz(image, meta);
    // as `series_attributes` writes the axes
    let unit = spatial_sizes_calibrated(image).then(|| "micrometer".to_string());
    Tile {
        name: file_name(input),
        path: input.to_path_buf(),
        series: Some(series),
        shape: [
            u64::from(meta.size_z.max(1)),
            u64::from(meta.size_y.max(1)),
            u64::from(meta.size_x.max(1)),
        ],
        scale: [z, y, x],
        units: [unit.clone(), unit.clone(), unit],
        stage: image.and_then(stage_reading),
        lavision: lavision_source(input, meta),
    }
}

/// The LaVision side of an input: its name, and the vendor XML in its
/// `ImageDescription`, when that is LaVision's.
fn lavision_source(input: &Path, meta: &ImageMetadata) -> Option<LaVisionSource> {
    let xml = meta
        .series_metadata
        .get("ImageDescription")
        .map(ToString::to_string)
        .and_then(|text| lavision::parse(&text));
    Some(LaVisionSource {
        file_name: file_name(input),
        xml,
    })
}

/// The stage position of an image's first plane, completed from its
/// `StageLabel`.
pub fn stage_reading(image: &OmeImage) -> Option<StageReading> {
    let plane = image
        .planes
        .iter()
        .find(|plane| plane.the_z == 0 && plane.the_c == 0 && plane.the_t == 0)
        .or_else(|| image.planes.first());
    let from_plane = plane.map_or([None; 3], |plane| {
        [plane.position_x, plane.position_y, plane.position_z]
    });
    let label = image.stage_label.as_ref();
    let from_label = label.map_or([None; 3], |label| [label.x, label.y, label.z]);
    let mut xyz = [None; 3];
    let mut parts = Vec::new();
    let mut label_axes = Vec::new();
    for axis in 0..3 {
        let finite = |value: Option<f64>| value.filter(|value| value.is_finite());
        if let Some(value) = finite(from_plane[axis]) {
            xyz[axis] = Some(value);
        } else if let Some(value) = finite(from_label[axis]) {
            xyz[axis] = Some(value);
            label_axes.push(["x", "y", "z"][axis]);
        }
    }
    if xyz.iter().all(Option::is_none) {
        return None;
    }
    if label_axes.len() < xyz.iter().flatten().count() {
        let plane = plane.expect("a plane gave a position");
        parts.push(format!(
            "Plane TheZ={} TheC={} TheT={}",
            plane.the_z, plane.the_c, plane.the_t
        ));
    }
    if !label_axes.is_empty() {
        let name = label
            .and_then(|label| label.name.as_deref())
            .map(|name| format!(" \"{name}\""))
            .unwrap_or_default();
        parts.push(format!("StageLabel{name} ({})", label_axes.join(", ")));
    }
    let shown = |value: Option<f64>| value.map_or("-".to_string(), |value| value.to_string());
    Some(StageReading {
        xyz,
        unit: "micrometer".to_string(),
        detail: format!(
            "{}: stage x {}, y {}, z {} (unit not reported by the reader; micrometer assumed)",
            parts.join(" + "),
            shown(xyz[0]),
            shown(xyz[1]),
            shown(xyz[2])
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bioformats_rs::common::ome_metadata::OmeStageLabel;
    use bioformats_rs::OmePlane;

    #[test]
    fn plane_positions_fall_back_to_the_stage_label() {
        let mut image = OmeImage::default();
        assert!(stage_reading(&image).is_none());
        image.planes = vec![
            OmePlane {
                the_z: 1,
                position_x: Some(9.0),
                ..Default::default()
            },
            OmePlane {
                the_z: 0,
                position_x: Some(10.0),
                position_y: Some(-4.0),
                ..Default::default()
            },
        ];
        image.stage_label = Some(OmeStageLabel {
            name: Some("A1".into()),
            x: Some(1.0),
            z: Some(7.0),
            ..Default::default()
        });
        let reading = stage_reading(&image).unwrap();
        assert_eq!(reading.xyz, [Some(10.0), Some(-4.0), Some(7.0)]);
        assert!(
            reading.detail.contains("Plane TheZ=0"),
            "{}",
            reading.detail
        );
        assert!(
            reading.detail.contains("StageLabel \"A1\" (z)"),
            "{}",
            reading.detail
        );
    }
}
