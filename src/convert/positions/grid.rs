//! The grid source: a row and column read from each tile's name, and one step
//! per grid direction (`--overlap` or `--pitch`).
//!
//! Tile `(row, column)` is placed at `row * pitch_row` along the image axis
//! `--grid-axes` gives rows (default `+y`) and `column * pitch_column` along
//! the one it gives columns (default `+x`), 0 on the third. The indices are
//! taken as they are, not rebased, so a sub-grid keeps its place.

use regex::Regex;

use super::{LengthUnit, Placement, SignedAxis, Tile};

/// The default name pattern: the last `<row> x <column>` in the name, spaces
/// around the `x` and its case free (`tile_03x04.zarr`, `scan[03 x 04]_c0`).
pub const DEFAULT_PATTERN: &str = r"(?P<row>\d+) *[xX] *(?P<col>\d+)";

/// How far apart neighbouring tiles are, per grid direction (row, column).
#[derive(Debug, Clone, PartialEq)]
pub enum GridStep {
    /// The tiles overlap by this much: pitch = tile extent - overlap.
    Overlap([f64; 2]),
    /// The distance between neighbouring tiles' first voxels.
    Pitch([f64; 2]),
}

#[derive(Debug, Clone)]
pub struct GridSpec {
    pub pattern: Regex,
    pub step: GridStep,
    pub unit: LengthUnit,
    /// The image axes rows and columns run along, with signs.
    pub axes: [SignedAxis; 2],
}

impl GridSpec {
    /// `pattern`: a regex with groups `row` and `col` (or `column`); the last
    /// match in the name counts. `axes`: e.g. `y,x` (the default) or `-y,x`.
    pub fn new(
        pattern: Option<&str>,
        step: GridStep,
        unit: LengthUnit,
        axes: Option<&str>,
    ) -> anyhow::Result<Self> {
        let pattern = Regex::new(pattern.unwrap_or(DEFAULT_PATTERN))?;
        let names: Vec<_> = pattern.capture_names().flatten().collect();
        if !names.contains(&"row") || !(names.contains(&"col") || names.contains(&"column")) {
            anyhow::bail!(
                "--grid-pattern needs named groups `row` and `col`, e.g. {DEFAULT_PATTERN}"
            );
        }
        let axes = axes
            .unwrap_or("y,x")
            .split(',')
            .map(|part| SignedAxis::parse(part, &[0, 1, 2]))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let axes: [SignedAxis; 2] = axes.try_into().map_err(|_| {
            anyhow::anyhow!("--grid-axes takes two axes, for rows and columns, e.g. y,x")
        })?;
        if axes[0].axis == axes[1].axis {
            anyhow::bail!("--grid-axes must name two different axes");
        }
        Ok(Self {
            pattern,
            step,
            unit,
            axes,
        })
    }

    pub fn axes_text(&self) -> String {
        format!("{},{}", self.axes[0], self.axes[1])
    }

    /// `[row, column]` from a name: the last match of the pattern.
    pub fn index(&self, name: &str) -> Option<[i64; 2]> {
        let captures = self.pattern.captures_iter(name).last()?;
        let row = captures.name("row")?.as_str().parse().ok()?;
        let column = captures
            .name("col")
            .or_else(|| captures.name("column"))?
            .as_str()
            .parse()
            .ok()?;
        Some([row, column])
    }

    /// Positions for every tile. The pitch along each grid direction must be
    /// one value for the whole batch, so the tiles must agree on their
    /// extent along it. Overlaps outside `0..tile` become warnings.
    pub fn place(
        &self,
        tiles: &[Tile],
        warnings: &mut Vec<String>,
    ) -> anyhow::Result<Vec<Placement>> {
        let Some(first) = tiles.first() else {
            return Ok(Vec::new());
        };
        let mut pitches = [0.0; 2];
        let mut described = Vec::new();
        for direction in 0..2 {
            // image axis in (z, y, x) order
            let axis = 2 - self.axes[direction].axis;
            for tile in tiles {
                if tile.shape[axis] != first.shape[axis]
                    || (tile.scale[axis] - first.scale[axis]).abs() > 1e-9 * first.scale[axis].abs()
                {
                    anyhow::bail!(
                        "{} and {} differ in size or voxel size along {}, so one grid pitch does not fit both",
                        first.label(),
                        tile.label(),
                        ["z", "y", "x"][axis]
                    );
                }
            }
            let extent = first.extent()[axis];
            let unit = first.units[axis].as_deref().unwrap_or("(no unit)");
            let (kind, given) = match &self.step {
                GridStep::Overlap(overlap) => ("overlap", overlap[direction]),
                GridStep::Pitch(pitch) => ("pitch", pitch[direction]),
            };
            let value = self.unit.to_tile(given, first, axis)?;
            let pitch = match self.step {
                GridStep::Overlap(_) => extent - value,
                GridStep::Pitch(_) => value,
            };
            // a gap, or neighbours stepping back over each other
            if pitch <= 0.0 || pitch > extent {
                warnings.push(format!(
                    "the {} {kind} {given} {} leaves a pitch of {pitch} {unit}, outside the tile's 0..{extent} along {}",
                    ["row", "column"][direction],
                    self.unit,
                    ["z", "y", "x"][axis]
                ));
            }
            pitches[direction] = pitch;
            described.push(format!(
                "{} {kind} {given} {} = {value} {unit}, pitch {pitch} {unit} along {}",
                ["row", "column"][direction],
                self.unit,
                self.axes[direction]
            ));
        }
        tiles
            .iter()
            .map(|tile| {
                let index = self.index(&tile.name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "{}: the grid pattern `{}` finds no row and column in the name",
                        tile.label(),
                        self.pattern
                    )
                })?;
                let mut translation = [0.0; 3];
                for direction in 0..2 {
                    let axis = self.axes[direction];
                    let sign = if axis.negate { -1.0 } else { 1.0 };
                    translation[2 - axis.axis] +=
                        sign * index[direction] as f64 * pitches[direction];
                }
                Ok(Placement {
                    translation: Some(translation),
                    source: "grid",
                    detail: format!(
                        "pattern {}: row {}, column {}; {}",
                        self.pattern,
                        index[0],
                        index[1],
                        described.join(", ")
                    ),
                    stage_axes: None,
                    grid_index: Some(index),
                    grid_axes: Some(self.axes_text()),
                })
            })
            .collect()
    }
}

/// `[row, column]` by the default pattern, for the neighbour checks of the
/// sources that give no grid themselves.
pub fn default_index(name: &str) -> Option<[i64; 2]> {
    static PATTERN: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let pattern = PATTERN.get_or_init(|| Regex::new(DEFAULT_PATTERN).expect("default pattern"));
    let captures = pattern.captures_iter(name).last()?;
    Some([captures["row"].parse().ok()?, captures["col"].parse().ok()?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::positions::tests::tile;

    #[test]
    fn default_pattern_reads_both_spellings_and_takes_the_last() {
        assert_eq!(default_index("tile_03x04.zarr"), Some([3, 4]));
        assert_eq!(default_index("scan[03 x 04]_C00.ome.tif"), Some([3, 4]));
        assert_eq!(default_index("2x_scan_07X12"), Some([7, 12]));
        assert_eq!(default_index("autofluorescence.zarr"), None);
    }

    #[test]
    fn grid_places_rows_and_columns_with_signs() {
        let tiles: Vec<Tile> = ["t_00x00", "t_00x01", "t_02x01"].map(tile).into();
        let grid = GridSpec::new(
            None,
            GridStep::Overlap([10.0, 20.0]),
            LengthUnit::Voxel,
            None,
        )
        .unwrap();
        let mut warnings = Vec::new();
        let placed = grid.place(&tiles, &mut warnings).unwrap();
        assert!(warnings.is_empty());
        let t: Vec<_> = placed.iter().map(|p| p.translation.unwrap()).collect();
        assert_eq!(t, [[0.0, 0.0, 0.0], [0.0, 0.0, 80.0], [0.0, 180.0, 80.0]]);

        let flipped = GridSpec::new(
            None,
            GridStep::Pitch([0.09, 0.08]),
            LengthUnit::parse("mm").unwrap(),
            Some("-y,x"),
        )
        .unwrap();
        let placed = flipped.place(&tiles, &mut warnings).unwrap();
        assert_eq!(placed[2].translation, Some([0.0, -180.0, 80.0]));
        assert_eq!(placed[2].provenance()["grid_axes"], "-y,x");

        let custom = GridSpec::new(
            Some(r"r(?P<row>\d+)_c(?P<col>\d+)"),
            GridStep::Overlap([150.0, 0.0]),
            LengthUnit::Voxel,
            None,
        )
        .unwrap();
        assert_eq!(custom.index("img_r2_c5.tif"), Some([2, 5]));
        let placed = custom
            .place(&["r0_c0", "r1_c0"].map(tile), &mut warnings)
            .unwrap();
        assert_eq!(placed[1].translation, Some([0.0, -50.0, 0.0]));
        assert_eq!(
            warnings.len(),
            1,
            "an overlap beyond the tile: {warnings:?}"
        );
        assert!(GridSpec::new(
            Some(r"(\d+)x(\d+)"),
            GridStep::Pitch([1.0, 1.0]),
            LengthUnit::Voxel,
            None
        )
        .is_err());
    }
}
