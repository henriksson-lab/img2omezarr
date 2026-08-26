use crate::convert::config::{ResolutionPolicy, SeriesSelection};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolutionLevel {
    pub index: usize,
    pub scale_x: u32,
    pub scale_y: u32,
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
                scale_x: scale as u32,
                scale_y: scale as u32,
                size_x: (size_x / scale).max(1),
                size_y: (size_y / scale).max(1),
                source: ResolutionSource::Generated,
            }
        })
        .collect()
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
        scale_x: rounded_scale(base_size_x, size_x),
        scale_y: rounded_scale(base_size_y, size_y),
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
}
