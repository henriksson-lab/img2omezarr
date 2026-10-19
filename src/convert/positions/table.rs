//! The table source (`--positions FILE`): one row per image.
//!
//! ```text
//! # comments and blank lines are skipped
//! image,series,z,y,x,unit
//! tile_00x00.zarr,,0,0,0,micrometer
//! tile_00x01.zarr,,0,0,1436.5,micrometer
//! ```
//!
//! Comma-separated, with a header naming the columns in any order. `image`,
//! `z`, `y`, `x` and `unit` are required, `series` is optional (empty means
//! series 0). `image` matches an image's path as given, its file name, or its
//! file name without the extension (`tile_00x00`). `unit` is `voxel` (level-0
//! voxels of that image) or a length (`micrometer`, `um`, `nm`, `mm`, ...).
//! `z`, `y`, `x` are image axes; `--stage-axes` reinterprets them as stage
//! axes (`x` column = stage x, and so on).

use std::path::{Path, PathBuf};

use super::{grid, LengthUnit, Placement, StageAxes, Tile};

#[derive(Debug, Clone)]
struct Row {
    line: usize,
    image: String,
    series: usize,
    zyx: [f64; 3],
    unit: LengthUnit,
    unit_text: String,
}

#[derive(Debug, Clone)]
pub struct PositionTable {
    path: PathBuf,
    rows: Vec<Row>,
}

impl PositionTable {
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| anyhow::anyhow!("reading {}: {error}", path.display()))?;
        Self::parse(path, &text)
    }

    pub fn parse(path: &Path, text: &str) -> anyhow::Result<Self> {
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(index, line)| (index + 1, line.trim()))
            .filter(|(_, line)| !line.is_empty() && !line.starts_with('#'));
        let (_, header) = lines
            .next()
            .ok_or_else(|| anyhow::anyhow!("{}: no header", path.display()))?;
        let header: Vec<String> = header
            .split(',')
            .map(|name| name.trim().to_ascii_lowercase())
            .collect();
        let column = |name: &str| header.iter().position(|column| column == name);
        let required = |name: &str| {
            column(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "{}: the header needs columns image, z, y, x, unit (and optionally series); no `{name}`",
                    path.display()
                )
            })
        };
        let (image, z, y, x, unit) = (
            required("image")?,
            required("z")?,
            required("y")?,
            required("x")?,
            required("unit")?,
        );
        let series = column("series");

        let mut rows = Vec::new();
        for (line, text) in lines {
            let fields: Vec<&str> = text.split(',').map(str::trim).collect();
            if fields.len() != header.len() {
                anyhow::bail!(
                    "{}:{line}: {} fields, the header has {}",
                    path.display(),
                    fields.len(),
                    header.len()
                );
            }
            let number = |index: usize| -> anyhow::Result<f64> {
                fields[index]
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        anyhow::anyhow!(
                            "{}:{line}: `{}` is not a number",
                            path.display(),
                            fields[index]
                        )
                    })
            };
            let series = match series.map(|index| fields[index]) {
                None | Some("") => 0,
                Some(text) => text.parse().map_err(|_| {
                    anyhow::anyhow!("{}:{line}: series `{text}` is not an index", path.display())
                })?,
            };
            rows.push(Row {
                line,
                image: fields[image].to_string(),
                series,
                zyx: [number(z)?, number(y)?, number(x)?],
                unit: LengthUnit::parse(fields[unit])
                    .map_err(|error| anyhow::anyhow!("{}:{line}: {error}", path.display()))?,
                unit_text: fields[unit].to_string(),
            });
        }
        Ok(Self {
            path: path.to_path_buf(),
            rows,
        })
    }

    /// The tile's row, converted to the tile's units. A tile without a row,
    /// or with more than one, is an error.
    pub fn place(&self, tile: &Tile, stage_axes: StageAxes) -> anyhow::Result<Placement> {
        let keys = image_keys(&tile.path);
        let series = tile.series.unwrap_or(0);
        let matches: Vec<&Row> = self
            .rows
            .iter()
            .filter(|row| row.series == series && keys.contains(&row.image))
            .collect();
        let row = match matches.as_slice() {
            [row] => *row,
            [] => anyhow::bail!("{}: no row for {}", self.path.display(), tile.label()),
            rows => anyhow::bail!(
                "{}: lines {} all match {}",
                self.path.display(),
                rows.iter()
                    .map(|row| row.line.to_string())
                    .collect::<Vec<_>>()
                    .join(", "),
                tile.label()
            ),
        };
        // the columns as stage (x, y, z), mapped back to image axes
        let image = stage_axes.image_zyx([Some(row.zyx[2]), Some(row.zyx[1]), Some(row.zyx[0])]);
        let mut translation = [0.0; 3];
        for axis in 0..3 {
            translation[axis] =
                row.unit
                    .to_tile(image[axis].expect("all columns given"), tile, axis)?;
        }
        Ok(Placement {
            translation: Some(translation),
            source: "table",
            detail: format!(
                "{} line {}: z {}, y {}, x {} {}",
                self.path.display(),
                row.line,
                row.zyx[0],
                row.zyx[1],
                row.zyx[2],
                row.unit_text
            ),
            stage_axes: Some(stage_axes),
            grid_index: grid::default_index(&tile.name),
            grid_axes: None,
        })
    }
}

/// The names a table row may use for an image: the path as given, the file
/// name, and the file name without its extension (`.ome.zarr`, `.zarr`,
/// `.ome.tif`, ...).
fn image_keys(path: &Path) -> Vec<String> {
    let mut keys = vec![path.display().to_string()];
    let name = super::file_name(path);
    keys.push(super::name_stem(&name));
    keys.insert(1, name);
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::positions::tests::tile;

    #[test]
    fn rows_match_by_path_name_or_stem_and_convert_units() {
        let table = PositionTable::parse(
            Path::new("p.csv"),
            "# a comment\n\nx, y, z, unit, image, series\n\
             1.5,0,0,mm,tile_00x01,\n\
             0,10,0,voxel,data/tile_01x00.ome.zarr,0\n\
             0,0,0,um,tile_00x00.zarr,1\n",
        )
        .unwrap();
        let mut a = tile("tile_00x01.zarr");
        a.path = PathBuf::from("/x/tile_00x01.zarr");
        let placed = table.place(&a, StageAxes::default()).unwrap();
        assert_eq!(placed.translation, Some([0.0, 0.0, 1500.0]));
        assert_eq!(placed.grid_index, Some([0, 1]));
        assert!(placed.detail.contains("line 4"), "{}", placed.detail);

        let mut b = tile("tile_01x00.ome.zarr");
        b.path = PathBuf::from("data/tile_01x00.ome.zarr");
        assert_eq!(
            table.place(&b, StageAxes::default()).unwrap().translation,
            Some([0.0, 10.0, 0.0])
        );
        let flipped = StageAxes::parse("x,-y,z").unwrap();
        assert_eq!(
            table.place(&b, flipped).unwrap().translation,
            Some([0.0, -10.0, 0.0])
        );

        // series 1 only
        let mut c = tile("tile_00x00.zarr");
        assert!(table.place(&c, StageAxes::default()).is_err());
        c.series = Some(1);
        assert!(table.place(&c, StageAxes::default()).is_ok());

        assert!(PositionTable::parse(Path::new("p.csv"), "image,z,y,x\n").is_err());
        assert!(
            PositionTable::parse(Path::new("p.csv"), "image,z,y,x,unit\na,1,2,3,furlong\n")
                .is_err()
        );
    }
}
