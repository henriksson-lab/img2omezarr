use crate::convert::config::{ResolutionPolicy, SeriesSelection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionLevel {
    pub index: usize,
    pub scale_z: u32,
    pub scale_x: u32,
    pub scale_y: u32,
    pub size_z: usize,
    pub size_x: usize,
    pub size_y: usize,
    pub source: ResolutionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionSource {
    Generated,
    Source(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesPlan {
    pub series: usize,
    pub levels: Vec<ResolutionLevel>,
}

pub fn selected_series(
    selection: &SeriesSelection,
    series_count: usize,
) -> anyhow::Result<Vec<usize>> {
    match selection {
        SeriesSelection::All => Ok((0..series_count).collect()),
        SeriesSelection::Indices(indices) => {
            for &index in indices {
                if index >= series_count {
                    anyhow::bail!("series index {index} is out of range for {series_count} series");
                }
            }
            Ok(indices.clone())
        }
    }
}

pub fn plan_resolutions(
    size_x: usize,
    size_y: usize,
    policy: ResolutionPolicy,
    source_resolution_count: usize,
) -> Vec<ResolutionLevel> {
    let levels = match policy {
        ResolutionPolicy::ExplicitLevels(n) => n.max(1),
        ResolutionPolicy::TargetMinSize(min) => calculated_levels(size_x, size_y, min),
        ResolutionPolicy::ExistingOrTargetMinSize(min) => {
            if source_resolution_count > 1 {
                source_resolution_count
            } else {
                calculated_levels(size_x, size_y, min)
            }
        }
    };
    (0..levels)
        .map(|index| {
            let scale = 2usize.pow(index as u32);
            ResolutionLevel {
                index,
                scale_z: 1,
                scale_x: scale as u32,
                scale_y: scale as u32,
                size_z: 1,
                size_x: (size_x / scale).max(1),
                size_y: (size_y / scale).max(1),
                source: ResolutionSource::Generated,
            }
        })
        .collect()
}

/// Most voxels a generated volume pyramid leaves in its coarsest level. A
/// viewer draws its fitted 3D overview from that whole level: newvolim packs it
/// into 4 GPU pages of 1 Mi voxels, chunk by chunk, so half that leaves room for
/// partly filled pages.
pub const MAX_COARSEST_VOLUME_VOXELS: usize = 1 << 21;

pub fn plan_volume_resolutions(
    size_x: usize,
    size_y: usize,
    size_z: usize,
    physical_size_xyz: [f64; 3],
    policy: ResolutionPolicy,
    source_resolution_count: usize,
) -> Vec<ResolutionLevel> {
    let explicit_levels = match policy {
        ResolutionPolicy::ExplicitLevels(n) => Some(n.max(1)),
        ResolutionPolicy::ExistingOrTargetMinSize(_) if source_resolution_count > 1 => {
            Some(source_resolution_count)
        }
        _ => None,
    };
    let target = match policy {
        ResolutionPolicy::TargetMinSize(min) | ResolutionPolicy::ExistingOrTargetMinSize(min) => {
            min.max(1) as usize
        }
        ResolutionPolicy::ExplicitLevels(_) => 1,
    };
    let base = [size_x.max(1), size_y.max(1), size_z.max(1)];
    let spacing = physical_size_xyz.map(|value| {
        if value.is_finite() && value > 0.0 {
            value
        } else {
            1.0
        }
    });
    let mut scales = [1usize; 3];
    let mut levels = Vec::new();
    loop {
        levels.push(ResolutionLevel {
            index: levels.len(),
            scale_x: scales[0] as u32,
            scale_y: scales[1] as u32,
            scale_z: scales[2] as u32,
            size_x: (base[0] / scales[0]).max(1),
            size_y: (base[1] / scales[1]).max(1),
            size_z: (base[2] / scales[2]).max(1),
            source: ResolutionSource::Generated,
        });
        if explicit_levels.is_some_and(|count| levels.len() >= count)
            || (explicit_levels.is_none()
                && base
                    .iter()
                    .zip(scales)
                    .all(|(&size, scale)| (size / scale).max(1) <= target)
                && levels.last().is_some_and(|level| {
                    level.size_x * level.size_y * level.size_z <= MAX_COARSEST_VOLUME_VOXELS
                }))
            || base
                .iter()
                .zip(scales)
                .all(|(&size, scale)| size / scale <= 1)
        {
            break;
        }
        let effective = std::array::from_fn::<_, 3, _>(|axis| spacing[axis] * scales[axis] as f64);
        let coarsest = effective.iter().copied().fold(0.0_f64, f64::max);
        let xy_can_coarsen = (base[0] / scales[0] > 1 || base[1] / scales[1] > 1)
            && effective[0].max(effective[1]) * 2.0 <= coarsest * 1.05;
        let z_can_coarsen = base[2] / scales[2] > 1 && effective[2] * 2.0 <= coarsest * 1.05;
        let any = xy_can_coarsen || z_can_coarsen;
        if xy_can_coarsen || !any {
            for axis in 0..2 {
                if base[axis] / scales[axis] > 1 {
                    scales[axis] = scales[axis].saturating_mul(2);
                }
            }
        }
        if base[2] / scales[2] > 1 && (z_can_coarsen || !any) {
            scales[2] = scales[2].saturating_mul(2);
        }
    }
    levels
}

pub fn source_resolution_level(
    index: usize,
    source_resolution: usize,
    base_size_x: usize,
    base_size_y: usize,
    size_x: usize,
    size_y: usize,
) -> ResolutionLevel {
    ResolutionLevel {
        index,
        scale_z: 1,
        scale_x: rounded_scale(base_size_x, size_x),
        scale_y: rounded_scale(base_size_y, size_y),
        size_z: 1,
        size_x: size_x.max(1),
        size_y: size_y.max(1),
        source: ResolutionSource::Source(source_resolution),
    }
}

fn rounded_scale(base: usize, level: usize) -> u32 {
    let level = level.max(1);
    ((base as f64 / level as f64).round() as u32).max(1)
}

pub fn calculated_levels(mut width: usize, mut height: usize, min_size: u32) -> usize {
    let min_size = min_size.max(1) as usize;
    let mut levels = 1;
    while (width > min_size || height > min_size) && width > 1 && height > 1 {
        levels += 1;
        width /= 2;
        height /= 2;
    }
    levels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_bioformats2raw_style_levels() {
        assert_eq!(calculated_levels(512, 512, 256), 2);
        assert_eq!(calculated_levels(4096, 2048, 256), 5);
        assert_eq!(calculated_levels(1, 512, 256), 1);
    }

    #[test]
    fn selected_series_checks_bounds() {
        assert_eq!(
            selected_series(&SeriesSelection::All, 3).unwrap(),
            [0, 1, 2]
        );
        assert!(selected_series(&SeriesSelection::Indices(vec![2, 3]), 3).is_err());
    }

    #[test]
    fn source_resolution_level_records_native_shape_and_scale() {
        let level = source_resolution_level(1, 1, 512, 256, 128, 64);
        assert_eq!(level.index, 1);
        assert_eq!(level.source, ResolutionSource::Source(1));
        assert_eq!((level.size_x, level.size_y), (128, 64));
        assert_eq!((level.scale_x, level.scale_y), (4, 4));
    }

    #[test]
    fn volume_levels_preserve_anisotropy_then_coarsen_all_axes() {
        let levels = plan_volume_resolutions(
            491,
            459,
            750,
            [0.5, 0.5, 2.0],
            ResolutionPolicy::TargetMinSize(256),
            1,
        );
        assert_eq!(
            levels
                .iter()
                .map(|level| {
                    (
                        [level.scale_z, level.scale_y, level.scale_x],
                        [level.size_z, level.size_y, level.size_x],
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                ([1, 1, 1], [750, 459, 491]),
                ([1, 2, 2], [750, 229, 245]),
                ([1, 4, 4], [750, 114, 122]),
                ([2, 8, 8], [375, 57, 61]),
                ([4, 16, 16], [187, 28, 30]),
            ]
        );
    }

    #[test]
    fn coarsest_volume_level_fits_a_viewer_overview() {
        // every axis is within 256 at 4×, but 156×206×202 is 6.5 M voxels
        let levels = plan_volume_resolutions(
            811,
            824,
            625,
            [2.0, 2.0, 2.0],
            ResolutionPolicy::TargetMinSize(256),
            1,
        );
        let sizes: Vec<_> = levels
            .iter()
            .map(|level| [level.size_z, level.size_y, level.size_x])
            .collect();
        assert_eq!(
            sizes,
            vec![
                [625, 824, 811],
                [312, 412, 405],
                [156, 206, 202],
                [78, 103, 101],
            ]
        );
        let coarsest = levels.last().unwrap();
        assert!(coarsest.size_x * coarsest.size_y * coarsest.size_z <= MAX_COARSEST_VOLUME_VOXELS);

        // a volume with nothing left to halve still ends
        let line =
            plan_volume_resolutions(1, 1, 1, [1.0; 3], ResolutionPolicy::TargetMinSize(0), 1);
        assert_eq!(line.len(), 1);
    }
}
