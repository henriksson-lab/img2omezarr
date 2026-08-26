use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisKind {
    Time,
    Channel,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisName {
    T,
    C,
    Z,
    Y,
    X,
}

impl AxisName {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::T => "t",
            Self::C => "c",
            Self::Z => "z",
            Self::Y => "y",
            Self::X => "x",
        }
    }

    pub fn kind(self) -> AxisKind {
        match self {
            Self::T => AxisKind::Time,
            Self::C => AxisKind::Channel,
            Self::Z | Self::Y | Self::X => AxisKind::Space,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axis {
    pub name: AxisName,
    pub length: usize,
    pub chunk: usize,
}

pub fn tczyx_axes(
    size_t: usize,
    size_c: usize,
    size_z: usize,
    size_y: usize,
    size_x: usize,
    tile_y: usize,
    tile_x: usize,
    chunk_z: usize,
) -> Vec<Axis> {
    vec![
        Axis {
            name: AxisName::T,
            length: size_t.max(1),
            chunk: 1,
        },
        Axis {
            name: AxisName::C,
            length: size_c.max(1),
            chunk: 1,
        },
        Axis {
            name: AxisName::Z,
            length: size_z.max(1),
            chunk: chunk_z.max(1).min(size_z.max(1)),
        },
        Axis {
            name: AxisName::Y,
            length: size_y.max(1),
            chunk: tile_y.max(1).min(size_y.max(1)),
        },
        Axis {
            name: AxisName::X,
            length: size_x.max(1),
            chunk: tile_x.max(1).min(size_x.max(1)),
        },
    ]
}

pub fn shape(axes: &[Axis]) -> Vec<usize> {
    axes.iter().map(|axis| axis.length).collect()
}

pub fn chunks(axes: &[Axis]) -> Vec<usize> {
    axes.iter().map(|axis| axis.chunk).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tczyx_axis_order_is_fixed() {
        let axes = tczyx_axes(2, 3, 4, 5, 6, 128, 128, 2);
        let names: Vec<_> = axes.iter().map(|axis| axis.name.as_str()).collect();
        assert_eq!(names, ["t", "c", "z", "y", "x"]);
        assert_eq!(shape(&axes), [2, 3, 4, 5, 6]);
        assert_eq!(chunks(&axes), [1, 1, 2, 5, 6]);
    }
}
