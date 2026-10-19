//! Tile positions: where each image's first voxel lies in a physical frame
//! that every tile of a sample shares. The contract is in
//! `docs/tile-positions.md`.
//!
//! A position is written as the level-0 `translation` of the image's
//! multiscale, in the image's own axes and units. It comes from one source,
//! the first that applies: a table (`--positions`), a grid read from the
//! names (`--overlap`/`--pitch`), a format's own metadata (LaVision's, from
//! the input or `--from-originals`), or the standard OME `Plane`/`StageLabel`
//! positions (`convert` only).
//! Whatever the source, the layout of a batch is checked before anything is
//! written (`check_layout`), and the group attributes record where the
//! position came from (`Placement::provenance`).

pub mod grid;
pub mod lavision;
#[cfg(feature = "core-bioformats")]
pub mod ome;
pub mod table;
pub mod zarr;

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub use grid::{GridSpec, GridStep};
pub use table::PositionTable;

/// Which positions `convert` and `set-positions` write.
#[derive(Debug, Clone, Default)]
pub struct PositionSettings {
    /// `--positions`: a table of positions per image.
    pub table: Option<PathBuf>,
    /// `--overlap`/`--pitch`: a grid read from the image names.
    pub grid: Option<GridSpec>,
    /// `--stage-axes`: replaces a source's default stage-to-image mapping.
    pub stage_axes: Option<StageAxes>,
    /// `--force`: write positions that fail the sanity checks.
    pub force: bool,
    /// `--no-positions`: write no translation (the attributes still say so).
    pub disabled: bool,
    /// `set-positions --from-originals`: read each image's original file.
    pub originals: Option<Originals>,
}

/// `set-positions --from-originals DIR [--originals-match REGEX]`: the files
/// the images were converted from, matched to them by name.
#[derive(Debug, Clone)]
pub struct Originals {
    pub dir: PathBuf,
    /// Only files whose name matches.
    pub matching: Option<regex::Regex>,
}

impl PositionSettings {
    pub fn user_source(&self) -> bool {
        self.table.is_some() || self.grid.is_some()
    }
}

/// One stage axis with a sign: image axis = sign * stage axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedAxis {
    /// 0 = x, 1 = y, 2 = z.
    pub axis: usize,
    pub negate: bool,
}

impl SignedAxis {
    pub fn parse(text: &str, allowed: &[usize]) -> anyhow::Result<Self> {
        let text = text.trim();
        let (negate, name) = match text.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, text.strip_prefix('+').unwrap_or(text)),
        };
        let axis = match name.to_ascii_lowercase().as_str() {
            "x" => 0,
            "y" => 1,
            "z" => 2,
            _ => anyhow::bail!("`{text}` is not an axis (x, y or z, optionally signed)"),
        };
        if !allowed.contains(&axis) {
            anyhow::bail!("axis `{text}` is not allowed here");
        }
        Ok(Self { axis, negate })
    }

    fn sign(self) -> f64 {
        if self.negate {
            -1.0
        } else {
            1.0
        }
    }
}

impl std::fmt::Display for SignedAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sign = if self.negate { "-" } else { "" };
        write!(f, "{sign}{}", ["x", "y", "z"][self.axis])
    }
}

/// `--stage-axes`: for image axes x, y and z in that order, the stage axis
/// (with a sign) that gives each. `x,y,z` is the identity, `x,-y,z` flips y,
/// `y,x,z` swaps x and y.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageAxes(pub [SignedAxis; 3]);

impl Default for StageAxes {
    fn default() -> Self {
        Self([0, 1, 2].map(|axis| SignedAxis {
            axis,
            negate: false,
        }))
    }
}

impl StageAxes {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let parts = text
            .split(',')
            .map(|part| SignedAxis::parse(part, &[0, 1, 2]))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let axes: [SignedAxis; 3] = parts.try_into().map_err(|_| {
            anyhow::anyhow!("--stage-axes takes three axes for image x, y, z, e.g. x,-y,z")
        })?;
        let mut seen = [false; 3];
        for axis in &axes {
            if std::mem::replace(&mut seen[axis.axis], true) {
                anyhow::bail!("--stage-axes `{text}` names a stage axis twice");
            }
        }
        Ok(Self(axes))
    }

    /// Image `(z, y, x)` from stage `(x, y, z)`. A missing stage value stays
    /// missing.
    pub fn image_zyx(&self, stage_xyz: [Option<f64>; 3]) -> [Option<f64>; 3] {
        let image_xyz = self
            .0
            .map(|axis| stage_xyz[axis.axis].map(|value| axis.sign() * value));
        [image_xyz[2], image_xyz[1], image_xyz[0]]
    }
}

impl std::fmt::Display for StageAxes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{},{},{}", self.0[0], self.0[1], self.0[2])
    }
}

/// A length unit as a power of ten of metres, for the NGFF length units and
/// the usual abbreviations. Powers keep conversions exact (1.5 mm is 1500 um).
pub fn unit_exponent(unit: &str) -> Option<i32> {
    Some(match unit.trim().to_lowercase().as_str() {
        "picometer" | "pm" => -12,
        "angstrom" | "å" => -10,
        "nanometer" | "nm" => -9,
        "micrometer" | "micron" | "um" | "µm" | "μm" => -6,
        "millimeter" | "mm" => -3,
        "centimeter" | "cm" => -2,
        "meter" | "m" => 0,
        _ => return None,
    })
}

/// The unit of a user-given length: level-0 voxels or a physical unit.
#[derive(Debug, Clone, PartialEq)]
pub enum LengthUnit {
    Voxel,
    Physical { name: String, exponent: i32 },
}

impl std::fmt::Display for LengthUnit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Voxel => write!(f, "voxel"),
            Self::Physical { name, .. } => write!(f, "{name}"),
        }
    }
}

impl LengthUnit {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let text = text.trim();
        if matches!(
            text.to_ascii_lowercase().as_str(),
            "voxel" | "voxels" | "vx" | "pixel" | "pixels" | "px"
        ) {
            return Ok(Self::Voxel);
        }
        unit_exponent(text)
            .map(|exponent| Self::Physical {
                name: text.to_string(),
                exponent,
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown unit `{text}` (voxel, nanometer, micrometer, millimeter, ...)"
                )
            })
    }

    /// `value` in this unit as a length along `axis` (0 = z, 1 = y, 2 = x) of
    /// `tile`, in the tile's own units.
    pub fn to_tile(&self, value: f64, tile: &Tile, axis: usize) -> anyhow::Result<f64> {
        match self {
            Self::Voxel => Ok(value * tile.scale[axis]),
            Self::Physical { name, exponent } => {
                let unit = tile.units[axis].as_deref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "{}: axis {} has no unit, so a length in {name} cannot be placed; give it in voxel",
                        tile.name,
                        ["z", "y", "x"][axis]
                    )
                })?;
                let tile_exponent = unit_exponent(unit).ok_or_else(|| {
                    anyhow::anyhow!("{}: axis unit `{unit}` is not a length", tile.name)
                })?;
                Ok(value * 10f64.powi(exponent - tile_exponent))
            }
        }
    }
}

/// What a position is placed against: one image (a tile), level 0.
#[derive(Debug, Clone)]
pub struct Tile {
    /// The name the table and grid match: the file name.
    pub name: String,
    /// The path as given (an input file, or an OME-Zarr image).
    pub path: PathBuf,
    /// The series in a multi-series file or collection.
    pub series: Option<usize>,
    /// Level-0 voxels, `(z, y, x)`; 1 along an axis the image lacks.
    pub shape: [u64; 3],
    /// Level-0 voxel size, `(z, y, x)`.
    pub scale: [f64; 3],
    /// The axes' units, `(z, y, x)`; `None` when uncalibrated.
    pub units: [Option<String>; 3],
    /// A stage position the reader reported (`convert` only).
    pub stage: Option<StageReading>,
    /// The LaVision XML of the file the tile came from, when that file was
    /// read (`convert`, or `set-positions --from-originals`).
    pub lavision: Option<lavision::LaVisionSource>,
}

impl Tile {
    pub fn label(&self) -> String {
        match self.series {
            Some(series) => format!("{} series {series}", self.name),
            None => self.name.clone(),
        }
    }

    /// The physical extent, `(z, y, x)`.
    pub fn extent(&self) -> [f64; 3] {
        [0, 1, 2].map(|axis| self.shape[axis] as f64 * self.scale[axis])
    }
}

/// A stage position as a reader reports it: stage `(x, y, z)`, in `unit`.
#[derive(Debug, Clone, PartialEq)]
pub struct StageReading {
    pub xyz: [Option<f64>; 3],
    pub unit: String,
    /// Where it came from, e.g. `Plane TheZ=0 TheC=0 TheT=0`.
    pub detail: String,
}

/// The position chosen for one tile, and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The level-0 translation, `(z, y, x)`, in the tile's units. `None`
    /// writes no translation.
    pub translation: Option<[f64; 3]>,
    /// `table`, `grid`, `lavision`, `ome-plane` or `none`.
    pub source: &'static str,
    pub detail: String,
    pub stage_axes: Option<StageAxes>,
    /// The tile's `[row, column]` when the source or the name gives one.
    pub grid_index: Option<[i64; 2]>,
    /// The grid's axis mapping (`grid` only).
    pub grid_axes: Option<String>,
}

impl Placement {
    pub fn none(detail: impl Into<String>) -> Self {
        Self {
            translation: None,
            source: "none",
            detail: detail.into(),
            stage_axes: None,
            grid_index: None,
            grid_axes: None,
        }
    }

    /// The `img2omezarr.positions` group attribute.
    pub fn provenance(&self) -> Value {
        let mut value = json!({ "source": self.source, "detail": self.detail });
        if let Some(stage_axes) = self.stage_axes {
            value["stage_axes"] = json!(stage_axes.to_string());
        }
        if let Some(grid_axes) = &self.grid_axes {
            value["grid_axes"] = json!(grid_axes);
        }
        if let Some(index) = self.grid_index {
            value["grid"] = json!(index);
        }
        value
    }
}

/// Positions for a batch of tiles from the source `settings` select: the
/// table, else the grid, else per tile its LaVision metadata, else its stage
/// reading. Then the sanity checks over the whole batch: their warnings go to
/// `warn`, and refuse the batch unless `--force`.
pub fn place_batch(
    tiles: &[Tile],
    settings: &PositionSettings,
    warn: &dyn Fn(&str),
) -> anyhow::Result<Vec<Placement>> {
    place_batch_with(
        tiles,
        settings,
        &lavision::ConfigIndex::from_tiles(tiles),
        warn,
    )
}

/// [`place_batch`], with the LaVision tile configurations to look tiles up
/// in given (e.g. from more files than the batch's).
pub fn place_batch_with(
    tiles: &[Tile],
    settings: &PositionSettings,
    configurations: &lavision::ConfigIndex,
    warn: &dyn Fn(&str),
) -> anyhow::Result<Vec<Placement>> {
    if settings.disabled {
        return Ok(tiles
            .iter()
            .map(|_| Placement::none("disabled with --no-positions"))
            .collect());
    }
    let mut warnings = Vec::new();
    let placements = if let Some(path) = &settings.table {
        let table = PositionTable::read(path)?;
        tiles
            .iter()
            .map(|tile| table.place(tile, settings.stage_axes.unwrap_or_default()))
            .collect::<anyhow::Result<Vec<_>>>()?
    } else if let Some(grid) = &settings.grid {
        if settings.stage_axes.is_some() {
            anyhow::bail!("--stage-axes does not apply to a grid; use --grid-axes");
        }
        grid.place(tiles, &mut warnings)?
    } else {
        let placements = tiles
            .iter()
            .map(|tile| {
                let from_stage = |note: Option<String>| {
                    place_from_stage(tile, settings.stage_axes).map(|mut placement| {
                        if let Some(note) = note {
                            placement.detail = format!("{}; LaVision: {note}", placement.detail);
                        }
                        placement
                    })
                };
                match lavision::place(tile, configurations, settings.stage_axes) {
                    Some(Ok(placement)) => Ok(placement),
                    Some(Err(reason)) => from_stage(Some(reason)),
                    None => from_stage(None),
                }
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        warnings.extend(mixed_sources(tiles, &placements));
        placements
    };
    warnings.extend(check_layout(tiles, &placements));
    enforce(&warnings, settings.force, warn)?;
    Ok(placements)
}

/// Tiles of one batch placed from different sources (some from LaVision
/// metadata, others from OME planes or not at all) are in different frames.
fn mixed_sources(tiles: &[Tile], placements: &[Placement]) -> Vec<String> {
    let lavision: Vec<_> = placements
        .iter()
        .filter(|placement| placement.source == "lavision")
        .collect();
    if lavision.is_empty() || lavision.len() == placements.len() {
        return Vec::new();
    }
    tiles
        .iter()
        .zip(placements)
        .filter(|(_, placement)| placement.source != "lavision")
        .map(|(tile, placement)| {
            format!(
                "{} has no LaVision position ({}: {}), unlike {} other tile(s) of the batch, so \
                 it would not share their frame",
                tile.label(),
                placement.source,
                placement.detail,
                lavision.len()
            )
        })
        .collect()
}

/// A tile's position from its stage reading (`ome-plane`), or none.
fn place_from_stage(tile: &Tile, stage_axes: Option<StageAxes>) -> anyhow::Result<Placement> {
    let Some(stage) = &tile.stage else {
        return Ok(Placement::none(
            "the reader reported no Plane or StageLabel position",
        ));
    };
    let mapping = stage_axes.unwrap_or_default();
    if tile.units.iter().any(Option::is_none) {
        return Ok(Placement::none(format!(
            "{} found, but the axes are uncalibrated, so it cannot be placed",
            stage.detail
        )));
    }
    let unit = LengthUnit::parse(&stage.unit)?;
    let image = mapping.image_zyx(stage.xyz);
    let mut missing = Vec::new();
    let mut translation = [0.0; 3];
    for axis in 0..3 {
        match image[axis] {
            Some(value) => translation[axis] = unit.to_tile(value, tile, axis)?,
            None => missing.push(["z", "y", "x"][axis]),
        }
    }
    let mut detail = format!("{}, {}", stage.detail, stage.unit);
    if !missing.is_empty() {
        detail.push_str(&format!("; no position for {}, 0 used", missing.join(", ")));
    }
    Ok(Placement {
        translation: Some(translation),
        source: "ome-plane",
        detail,
        stage_axes: Some(mapping),
        grid_index: grid::default_index(&tile.name),
        grid_axes: None,
    })
}

/// Warn, and refuse unless `force`.
pub fn enforce(warnings: &[String], force: bool, warn: &dyn Fn(&str)) -> anyhow::Result<()> {
    for warning in warnings {
        warn(&format!("tile positions: {warning}"));
    }
    if !warnings.is_empty() && !force {
        anyhow::bail!(
            "{} tile position check(s) failed:\n  {}\npass --force to write these positions anyway",
            warnings.len(),
            warnings.join("\n  ")
        );
    }
    Ok(())
}

/// The sanity checks on a batch's positions:
///
/// - two tiles at the same position (within half a voxel on every axis);
/// - neighbours that do not overlap, i.e. a gap (overlap < 0), or whose step
///   runs the other way from the other neighbours' (an overlap beyond the
///   tile). Neighbours are grid neighbours when every placed tile has a grid
///   index, else for each tile and axis the nearest tile further along it
///   that lies within half a tile on the other axes.
pub fn check_layout(tiles: &[Tile], placements: &[Placement]) -> Vec<String> {
    // (tile, translation, grid index) of the tiles that have a position
    type Placed<'a> = (&'a Tile, [f64; 3], Option<[i64; 2]>);
    let placed: Vec<Placed> = tiles
        .iter()
        .zip(placements)
        .filter_map(|(tile, placement)| {
            placement
                .translation
                .map(|translation| (tile, translation, placement.grid_index))
        })
        .collect();
    let mut warnings = Vec::new();
    let fmt = |t: [f64; 3]| format!("({}, {}, {})", t[0], t[1], t[2]);

    for (i, (a, ta, _)) in placed.iter().enumerate() {
        for (b, tb, _) in &placed[i + 1..] {
            if (0..3).all(|axis| (ta[axis] - tb[axis]).abs() < 0.5 * a.scale[axis]) {
                warnings.push(format!(
                    "{} and {} have the same position {}",
                    a.label(),
                    b.label(),
                    fmt(*ta)
                ));
            }
        }
    }

    let gap = |a: &Tile, b: &Tile, axis: usize, step: f64| -> Option<String> {
        let overlap = a.extent()[axis] - step.abs();
        (overlap < -0.5 * a.scale[axis]).then(|| {
            format!(
                "{} and {} are {} apart along {}, beyond the tile's {}: a gap of {}",
                a.label(),
                b.label(),
                step.abs(),
                ["z", "y", "x"][axis],
                a.extent()[axis],
                -overlap
            )
        })
    };

    if !placed.is_empty() && placed.iter().all(|(_, _, index)| index.is_some()) {
        // grid neighbours: per grid direction, every step should run along the
        // same image axis, the same way
        for direction in 0..2 {
            let mut steps = Vec::new();
            for (a, ta, ia) in &placed {
                let mut next = ia.expect("grid index");
                next[direction] += 1;
                for (b, tb, ib) in &placed {
                    if *ib == Some(next) {
                        let d = [0, 1, 2].map(|axis| tb[axis] - ta[axis]);
                        let axis = (0..3)
                            .max_by(|&p, &q| {
                                (d[p] / a.extent()[p])
                                    .abs()
                                    .total_cmp(&(d[q] / a.extent()[q]).abs())
                            })
                            .expect("three axes");
                        steps.push((*a, *b, axis, d[axis]));
                    }
                }
            }
            let mut votes = std::collections::BTreeMap::new();
            for (_, _, axis, step) in &steps {
                if *step != 0.0 {
                    *votes.entry((*axis, *step > 0.0)).or_insert(0usize) += 1;
                }
            }
            let usual = votes
                .iter()
                .max_by_key(|(_, count)| **count)
                .map(|(key, _)| *key);
            let name = ["row", "column"][direction];
            for (a, b, axis, step) in steps {
                if step == 0.0 {
                    continue;
                }
                if let Some((usual_axis, usual_positive)) = usual {
                    if (axis, step > 0.0) != (usual_axis, usual_positive) {
                        warnings.push(format!(
                            "{} -> {} (next {name}) steps {step} along {}, unlike the other {name} \
                             neighbours ({}{}): an overlap beyond the tile, or a misplaced tile",
                            a.label(),
                            b.label(),
                            ["z", "y", "x"][axis],
                            if usual_positive { "+" } else { "-" },
                            ["z", "y", "x"][usual_axis]
                        ));
                        continue;
                    }
                }
                warnings.extend(gap(a, b, axis, step));
            }
        }
    } else {
        for (a, ta, _) in &placed {
            for axis in 0..3 {
                let nearest = placed
                    .iter()
                    .filter(|(b, tb, _)| {
                        tb[axis] - ta[axis] >= 0.5 * a.scale[axis]
                            && (0..3).filter(|&other| other != axis).all(|other| {
                                (tb[other] - ta[other]).abs()
                                    < 0.5 * a.extent()[other].max(b.extent()[other])
                            })
                    })
                    .min_by(|(_, p, _), (_, q, _)| p[axis].total_cmp(&q[axis]));
                if let Some((b, tb, _)) = nearest {
                    warnings.extend(gap(a, b, axis, tb[axis] - ta[axis]));
                }
            }
        }
    }
    warnings
}

/// A file name without its image extension: `.ome.zarr`, `.zarr`,
/// `.ome.tiff`, `.ome.tif` (any case), else the last extension.
pub fn name_stem(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    [".ome.zarr", ".zarr", ".ome.tiff", ".ome.tif"]
        .iter()
        .find(|suffix| lower.ends_with(*suffix))
        .map(|suffix| name[..name.len() - suffix.len()].to_string())
        .unwrap_or_else(|| {
            Path::new(name).file_stem().map_or_else(
                || name.to_string(),
                |stem| stem.to_string_lossy().into_owned(),
            )
        })
}

/// The name a table or grid matches for `path`: its file name.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn tile(name: &str) -> Tile {
        Tile {
            name: name.to_string(),
            path: PathBuf::from(name),
            series: None,
            shape: [10, 100, 100],
            scale: [2.0, 1.0, 1.0],
            units: [0, 1, 2].map(|_| Some("micrometer".to_string())),
            stage: None,
            lavision: None,
        }
    }

    fn placed(t: [f64; 3], index: Option<[i64; 2]>) -> Placement {
        Placement {
            translation: Some(t),
            source: "table",
            detail: String::new(),
            stage_axes: None,
            grid_index: index,
            grid_axes: None,
        }
    }

    #[test]
    fn stage_axes_map_stage_to_image() {
        let axes = StageAxes::parse("x,-y,z").unwrap();
        assert_eq!(axes.to_string(), "x,-y,z");
        assert_eq!(
            axes.image_zyx([Some(1.0), Some(2.0), Some(3.0)]),
            [Some(3.0), Some(-2.0), Some(1.0)]
        );
        let swapped = StageAxes::parse("y,x,-z").unwrap();
        assert_eq!(
            swapped.image_zyx([Some(1.0), Some(2.0), None]),
            [None, Some(1.0), Some(2.0)]
        );
        assert!(StageAxes::parse("x,x,z").is_err());
        assert!(StageAxes::parse("x,y").is_err());
    }

    #[test]
    fn units_convert_to_the_tile_axis_unit() {
        let t = tile("a");
        assert_eq!(
            LengthUnit::parse("voxel")
                .unwrap()
                .to_tile(3.0, &t, 0)
                .unwrap(),
            6.0
        );
        assert_eq!(
            LengthUnit::parse("mm")
                .unwrap()
                .to_tile(1.5, &t, 2)
                .unwrap(),
            1500.0
        );
        let mut uncalibrated = t.clone();
        uncalibrated.units[2] = None;
        assert!(LengthUnit::parse("um")
            .unwrap()
            .to_tile(1.0, &uncalibrated, 2)
            .is_err());
    }

    #[test]
    fn layout_checks_find_duplicates_gaps_and_reversed_steps() {
        let tiles: Vec<Tile> = ["a", "b", "c"].map(tile).into();
        let ok = [
            placed([0.0, 0.0, 0.0], Some([0, 0])),
            placed([0.0, 0.0, 90.0], Some([0, 1])),
            placed([0.0, 0.0, 180.0], Some([0, 2])),
        ];
        assert!(check_layout(&tiles, &ok).is_empty());

        let duplicate = [
            ok[0].clone(),
            ok[1].clone(),
            placed([0.0, 0.2, 0.0], Some([0, 2])),
        ];
        let warnings = check_layout(&tiles, &duplicate);
        assert!(
            warnings.iter().any(|w| w.contains("same position")),
            "{warnings:?}"
        );

        let gap = [
            ok[0].clone(),
            ok[1].clone(),
            placed([0.0, 0.0, 300.0], Some([0, 2])),
        ];
        let warnings = check_layout(&tiles, &gap);
        assert!(
            warnings.iter().any(|w| w.contains("gap of 110")),
            "{warnings:?}"
        );

        let mut four: Vec<Tile> = tiles.clone();
        four.push(tile("d"));
        let reversed = [
            ok[0].clone(),
            ok[1].clone(),
            ok[2].clone(),
            placed([0.0, 0.0, 100.0], Some([0, 3])),
        ];
        let warnings = check_layout(&four, &reversed);
        assert!(
            warnings.iter().any(|w| w.contains("unlike the other")),
            "{warnings:?}"
        );

        // without grid indices: the nearest tile along each axis
        let free = [
            placed([0.0, 0.0, 0.0], None),
            placed([0.0, 5.0, 150.0], None),
            placed([0.0, 0.0, 400.0], None),
        ];
        let warnings = check_layout(&tiles, &free);
        assert_eq!(warnings.len(), 2, "{warnings:?}");
    }
}
