//! The LaVision source: stage positions from the vendor XML that LaVision's
//! UltraMicroscope (Imspector) writes into its OME-TIFFs.
//!
//! The OME model has no slot for these. The files carry no `<Plane>` and no
//! `<StageLabel>`, so Bio-Formats (and bioformats-rs, which follows it)
//! reports no position. The positions sit in `OME/Image/CustomAttributes`,
//! in LaVision's own namespace:
//!
//! ```xml
//! <Image Name="13-01-56_FOS_UltraII[00 x 01]_C00.ome.tif" ...>
//!   <ca:CustomAttributes>
//!     <Offset Offset_0="1395.453979" Offset_1="1584.480957" Offset_2="0.0" Offset_3="0.0"/>
//!     <IsPartOfMosaic IsPartOfMosaic="1"/>
//!     <TileConfiguration TileConfiguration="2
//! 13-01-56_FOS_UltraII[00 x 00]_C00.ome.tif;;(-72.895996, 1584.480957)
//! 13-01-56_FOS_UltraII[00 x 01]_C00.ome.tif;;(1395.453979, 1584.480957)
//! ..."/>
//! ```
//!
//! - `<Offset>` is the stage position (x, y, z; micrometres) of the file the
//!   XML was written for, which `<Image Name>` names.
//! - `<TileConfiguration>` is the whole mosaic, the same in every file of it:
//!   a dimension count, then one `name;;(x, y[, z])` line per file. A third
//!   value, where there is one, numbers the channel (`Filter0000` 0,
//!   `Filter0001` 1), so it is not used as a position.
//!
//! A file's position is looked up by **its own name** in the configuration
//! (of its own XML, or of any file of the batch), and only otherwise taken
//! from `<Offset>`, and then only when the XML names this file. So a tile
//! whose XML was copied from another (some acquisitions store it in one file
//! only, and converters borrow it) still gets its own position, and never
//! the other file's. A file that is not part of a mosaic
//! (`IsPartOfMosaic="0"`, e.g. a single autofluorescence stack) has an
//! all-zero `<Offset>` that is no stage position, so it gets none.
//!
//! The default stage-to-image mapping is `x,-y,-z`: stage x runs along image
//! x, and stage y against image y (mosaic rows step down in stage y and are
//! stacked along +y). Verified against a stitched mosaic (osf-cztq2,
//! `docs/tile-positions.md`). Stage z decreases plane by plane
//! (`PhysicalUnit` < 0 on the z axis), hence `-z`; `Offset_2` is 0 on every
//! file seen, so z has not been checked against data.

use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::{grid, LengthUnit, Placement, SignedAxis, StageAxes, Tile};

/// `x,-y,-z`: see the module documentation.
pub const DEFAULT_STAGE_AXES: StageAxes = StageAxes([
    SignedAxis {
        axis: 0,
        negate: false,
    },
    SignedAxis {
        axis: 1,
        negate: true,
    },
    SignedAxis {
        axis: 2,
        negate: true,
    },
]);

/// What a tile's LaVision XML says, parsed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LaVisionXml {
    /// `<Image Name>`: the file the XML was written for.
    pub image_name: Option<String>,
    /// `<Offset Offset_0/_1/_2>`: that file's stage x, y, z (micrometres).
    pub offset: [Option<f64>; 3],
    /// `<IsPartOfMosaic>`, when present.
    pub part_of_mosaic: Option<bool>,
    /// `<TileConfiguration>`: every file of the mosaic, by name.
    pub tiles: Vec<ConfigTile>,
}

/// One line of `<TileConfiguration>`: stage x and y, micrometres.
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigTile {
    pub file_name: String,
    pub xy: [f64; 2],
}

/// The LaVision side of one tile: the name it is looked up by (the original
/// file's) and its XML, when it has one.
#[derive(Debug, Clone, PartialEq)]
pub struct LaVisionSource {
    pub file_name: String,
    pub xml: Option<LaVisionXml>,
}

/// The configurations of a batch: file name -> stage x, y, with the file
/// whose XML gave it.
#[derive(Debug, Clone, Default)]
pub struct ConfigIndex {
    entries: BTreeMap<String, ([f64; 2], String)>,
    /// Names listed twice with different positions.
    conflicts: Vec<String>,
}

impl ConfigIndex {
    pub fn add(&mut self, xml: &LaVisionXml, from: &str) {
        for tile in &xml.tiles {
            match self.entries.get(&tile.file_name) {
                Some((xy, _)) if *xy != tile.xy => {
                    if !self.conflicts.contains(&tile.file_name) {
                        self.conflicts.push(tile.file_name.clone());
                    }
                }
                Some(_) => {}
                None => {
                    self.entries
                        .insert(tile.file_name.clone(), (tile.xy, from.to_string()));
                }
            }
        }
    }

    /// From every tile of a batch that has LaVision XML.
    pub fn from_tiles(tiles: &[Tile]) -> Self {
        let mut index = Self::default();
        for tile in tiles {
            if let Some(LaVisionSource {
                file_name,
                xml: Some(xml),
            }) = &tile.lavision
            {
                index.add(xml, file_name);
            }
        }
        index
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Configurations that list one file at two positions.
    pub fn conflicts(&self) -> &[String] {
        &self.conflicts
    }
}

/// A stage position (x, y, z; micrometres) and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub struct StagePosition {
    pub xyz: [Option<f64>; 3],
    pub detail: String,
}

/// The stage position of the file `source` names: its own line in a
/// configuration, else its own `<Offset>`. `None`, with the reason, when
/// neither applies.
pub fn stage_position(
    source: &LaVisionSource,
    index: &ConfigIndex,
) -> Result<StagePosition, String> {
    let name = &source.file_name;
    let own = source
        .xml
        .as_ref()
        .filter(|xml| xml.image_name.as_deref() == Some(name.as_str()));
    // z is only in <Offset>, so only a file's own XML gives it
    let z = own.and_then(|xml| xml.offset[2]);
    let z_note = match (own, z) {
        (Some(_), Some(_)) => String::new(),
        (Some(_), None) => "; the XML gives no Offset_2, z 0 used".to_string(),
        (None, _) => "; z is only in a file's own <Offset>, 0 used".to_string(),
    };
    if let Some((xy, from)) = index.entries.get(name) {
        if index.conflicts.contains(name) {
            return Err(format!(
                "the TileConfigurations of the batch list {name} at different positions"
            ));
        }
        let mut detail = format!(
            "TileConfiguration line for {name} (from {from}'s XML): stage x {}, y {}",
            xy[0], xy[1]
        );
        if let Some(xml) = own {
            if let (Some(x), Some(y)) = (xml.offset[0], xml.offset[1]) {
                if (x - xy[0]).abs() > 1e-3 || (y - xy[1]).abs() > 1e-3 {
                    detail.push_str(&format!(
                        "; its own <Offset> differs (x {x}, y {y}), the TileConfiguration is used"
                    ));
                }
            }
        }
        detail.push_str(&z_note);
        return Ok(StagePosition {
            xyz: [Some(xy[0]), Some(xy[1]), Some(z.unwrap_or(0.0))],
            detail,
        });
    }
    let Some(xml) = &source.xml else {
        return Err(format!(
            "{name} has no LaVision XML, and no TileConfiguration of the batch lists it"
        ));
    };
    if own.is_none() {
        return Err(format!(
            "{name}'s LaVision XML was written for {}, and its TileConfiguration does not list \
             {name}, so its <Offset> belongs to another file",
            xml.image_name.as_deref().unwrap_or("an unnamed image")
        ));
    }
    if xml.part_of_mosaic == Some(false) {
        return Err(format!(
            "{name} is not part of a mosaic (IsPartOfMosaic 0): its <Offset> is no stage position"
        ));
    }
    if xml.offset[0].is_none() || xml.offset[1].is_none() {
        return Err(format!("{name}'s LaVision XML has no <Offset> x and y"));
    }
    Ok(StagePosition {
        xyz: [xml.offset[0], xml.offset[1], Some(z.unwrap_or(0.0))],
        detail: format!(
            "<Offset> of {name}'s own XML: stage x {}, y {}{z_note}",
            xml.offset[0].expect("checked"),
            xml.offset[1].expect("checked")
        ),
    })
}

/// A tile's placement from LaVision metadata, or why there is none (`Err`,
/// not an error: the next source is tried). `None` when the tile has no
/// LaVision side at all.
pub fn place(
    tile: &Tile,
    index: &ConfigIndex,
    stage_axes: Option<StageAxes>,
) -> Option<Result<Placement, String>> {
    let source = tile.lavision.as_ref()?;
    if source.xml.is_none() && !index.entries.contains_key(&source.file_name) {
        // nothing LaVision about this file
        return None;
    }
    let position = match stage_position(source, index) {
        Ok(position) => position,
        Err(reason) => return Some(Err(reason)),
    };
    if tile.units.iter().any(Option::is_none) {
        return Some(Err(format!(
            "{}, but the axes are uncalibrated, so it cannot be placed",
            position.detail
        )));
    }
    let mapping = stage_axes.unwrap_or(DEFAULT_STAGE_AXES);
    let image = mapping.image_zyx(position.xyz);
    let unit = LengthUnit::parse("micrometer").expect("a known unit");
    let mut translation = [0.0; 3];
    for axis in 0..3 {
        let value = image[axis].expect("every stage axis is set");
        translation[axis] = match unit.to_tile(value, tile, axis) {
            // + 0.0: a negated zero is written as 0
            Ok(value) => value + 0.0,
            Err(error) => return Some(Err(error.to_string())),
        };
    }
    Some(Ok(Placement {
        translation: Some(translation),
        source: "lavision",
        detail: format!("{}, micrometer", position.detail),
        stage_axes: Some(mapping),
        grid_index: grid::default_index(&tile.name),
        grid_axes: None,
    }))
}

/// True when `xml` looks like LaVision's: both its stage-control prefix
/// (`xyz-Table`, dozens of times in a real file) and an `<Offset` element.
pub fn looks_like_lavision(xml: &str) -> bool {
    xml.contains("xyz-Table") && xml.contains("<Offset ")
}

/// Parse the LaVision elements out of an OME-XML document; `None` when it is
/// not LaVision's.
pub fn parse(xml: &str) -> Option<LaVisionXml> {
    if !looks_like_lavision(xml) {
        return None;
    }
    let mut parsed = LaVisionXml::default();
    if let Some(tag) = find_element(xml, "<Image ") {
        parsed.image_name = attribute(tag, "Name").map(unescape);
    }
    if let Some(tag) = find_element(xml, "<Offset ") {
        for (axis, name) in ["Offset_0", "Offset_1", "Offset_2"].iter().enumerate() {
            parsed.offset[axis] = attribute(tag, name).and_then(|value| value.parse().ok());
        }
    }
    if let Some(tag) = find_element(xml, "<IsPartOfMosaic ") {
        parsed.part_of_mosaic = attribute(tag, "IsPartOfMosaic").map(|value| value.trim() != "0");
    }
    if let Some(tag) = find_element(xml, "<TileConfiguration ") {
        if let Some(body) = attribute(tag, "TileConfiguration") {
            parsed.tiles = parse_tile_configuration(body);
        }
    }
    Some(parsed)
}

/// The text of the first element starting with `open`, through its `>`.
fn find_element<'a>(xml: &'a str, open: &str) -> Option<&'a str> {
    let start = xml.find(open)?;
    let rest = &xml[start..];
    let end = rest.find('>')?;
    Some(&rest[..=end])
}

/// `name="value"` out of one element's text, not matching a longer name
/// (`Offset_0` in `Offset_01`) or the element's own name.
fn attribute<'a>(element: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(hit) = element[from..].find(name) {
        let at = from + hit;
        let before_ok = element[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_whitespace);
        let after = &element[at + name.len()..];
        if before_ok && after.starts_with("=\"") {
            let value = &after[2..];
            return value.find('"').map(|end| &value[..end]);
        }
        from = at + name.len();
    }
    None
}

fn unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The body of `TileConfiguration`: a dimension count line, then
/// `name;;(x, y)` or `name;;(x, y, channel)` per file. Other lines are
/// skipped.
fn parse_tile_configuration(body: &str) -> Vec<ConfigTile> {
    let mut tiles = Vec::new();
    for line in body.lines() {
        let Some((name, coordinates)) = line.trim().split_once(";;") else {
            continue;
        };
        let Some(inner) = coordinates
            .trim()
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
        else {
            continue;
        };
        let values: Vec<f64> = inner
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        if values.len() == 2 || values.len() == 3 {
            tiles.push(ConfigTile {
                file_name: unescape(name.trim()),
                xy: [values[0], values[1]],
            });
        }
    }
    tiles
}

/// The `ImageDescription` (tag 270) of a TIFF's first IFD, classic or
/// BigTIFF, without reading the rest of the file. `--from-originals` uses
/// this rather than opening the whole OME-TIFF: a LaVision tile has an IFD
/// per plane spread through gigabytes, and walking them all takes 20 to 60 s
/// a file on a network disk, while the XML is in the first IFD. `None` when
/// the file is not a TIFF or has no description there.
pub fn first_image_description(path: &Path) -> anyhow::Result<Option<String>> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| anyhow::anyhow!("opening {}: {error}", path.display()))?;
    let mut header = [0u8; 16];
    if file.read(&mut header)? < 8 {
        return Ok(None);
    }
    let little = match &header[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Ok(None),
    };
    let u16_at = |bytes: &[u8]| {
        let pair = [bytes[0], bytes[1]];
        if little {
            u16::from_le_bytes(pair)
        } else {
            u16::from_be_bytes(pair)
        }
    };
    let u32_at = |bytes: &[u8]| {
        let quad = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if little {
            u32::from_le_bytes(quad)
        } else {
            u32::from_be_bytes(quad)
        }
    };
    let u64_at = |bytes: &[u8]| {
        let mut eight = [0u8; 8];
        eight.copy_from_slice(&bytes[..8]);
        if little {
            u64::from_le_bytes(eight)
        } else {
            u64::from_be_bytes(eight)
        }
    };
    let big = match u16_at(&header[2..4]) {
        42 => false,
        43 => true,
        _ => return Ok(None),
    };
    let ifd = if big {
        u64_at(&header[8..16])
    } else {
        u64::from(u32_at(&header[4..8]))
    };
    file.seek(SeekFrom::Start(ifd))?;
    let (count, entry_size) = if big {
        let mut count = [0u8; 8];
        file.read_exact(&mut count)?;
        (u64_at(&count), 20usize)
    } else {
        let mut count = [0u8; 2];
        file.read_exact(&mut count)?;
        (u64::from(u16_at(&count)), 12usize)
    };
    if count > 4096 {
        anyhow::bail!(
            "{}: implausible first IFD ({count} entries)",
            path.display()
        );
    }
    let mut entries = vec![0u8; count as usize * entry_size];
    file.read_exact(&mut entries)?;
    for entry in entries.chunks(entry_size) {
        if u16_at(&entry[0..2]) != 270 {
            continue;
        }
        let (length, inline_size, value) = if big {
            (u64_at(&entry[4..12]), 8, &entry[12..20])
        } else {
            (u64::from(u32_at(&entry[4..8])), 4, &entry[8..12])
        };
        if length > 1 << 30 {
            anyhow::bail!("{}: implausible ImageDescription length", path.display());
        }
        let bytes = if length as usize <= inline_size {
            value[..length as usize].to_vec()
        } else {
            let offset = if big {
                u64_at(value)
            } else {
                u64::from(u32_at(value))
            };
            file.seek(SeekFrom::Start(offset))?;
            let mut bytes = vec![0u8; length as usize];
            file.read_exact(&mut bytes)?;
            bytes
        };
        let text = String::from_utf8_lossy(&bytes);
        return Ok(Some(text.trim_end_matches('\0').to_string()));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::positions::tests::tile;

    /// The shape of a real UltraII file's XML, trimmed.
    pub(crate) fn sample(image_name: &str, offset: [f64; 2]) -> String {
        format!(
            r#"<OME xmlns:ca="http://www.openmicroscopy.org/Schemas/CA/2008-02">
  <Image ID="Image:1" Name="{image_name}">
    <ca:CustomAttributes>
      <Offset Offset_0="{}" Offset_1="{}" Offset_2="0.0" Offset_3="0.0"/>
      <Axis0 AxisName="xyz-Table Z" Offset="-1336.662109" PhysicalUnit="-1.600000" Steps="3369"/>
      <IsPartOfMosaic IsPartOfMosaic="1"/>
      <TileConfiguration TileConfiguration="3
x[05 x 05]_C00_UltraII Filter0000.ome.tif;;(-1861.449219, -5300.662598,0.000000)
x[05 x 05]_C00_UltraII Filter0001.ome.tif;;(-1861.449219, -5300.662598,1.000000)
x[05 x 06]_C00_UltraII Filter0000.ome.tif;;(-1270.599365, -5300.662598,0.000000)
x[04 x 05]_C00_UltraII Filter0000.ome.tif;;(-1861.449219, -3393.562500,0.000000)
"/>
    </ca:CustomAttributes>
  </Image>
  <ca:CustomAttributes><PropArray><xyz-Table_X_Overlap Value="65.650002"/></PropArray></ca:CustomAttributes>
</OME>"#,
            offset[0], offset[1]
        )
    }

    fn lavision_tile(name: &str, xml: Option<String>) -> Tile {
        let mut t = tile(name);
        t.scale = [1.6, 1.625, 1.625];
        t.lavision = Some(LaVisionSource {
            file_name: name.to_string(),
            xml: xml.as_deref().and_then(parse),
        });
        t
    }

    #[test]
    fn parses_offset_name_mosaic_flag_and_configuration() {
        let xml = parse(&sample("a.ome.tif", [1.5, -2.5])).unwrap();
        assert_eq!(xml.image_name.as_deref(), Some("a.ome.tif"));
        assert_eq!(xml.offset, [Some(1.5), Some(-2.5), Some(0.0)]);
        assert_eq!(xml.part_of_mosaic, Some(true));
        assert_eq!(xml.tiles.len(), 4);
        assert_eq!(xml.tiles[2].xy, [-1270.599365, -5300.662598]);
        // a two-dimensional configuration (no channel number)
        let two = parse_tile_configuration("2\na.tif;;(-72.895996, 1584.480957)\n");
        assert_eq!(two[0].xy, [-72.895996, 1584.480957]);
        assert!(parse("<OME><Image Name=\"a\"/></OME>").is_none());
        assert!(parse("<OME><Offset Offset_0=\"1\"/></OME>").is_none());
        assert_eq!(
            attribute(r#"<Offset Offset_01="9.0" Offset_0="1.5"/>"#, "Offset_0"),
            Some("1.5")
        );
    }

    #[test]
    fn a_tile_with_borrowed_xml_gets_its_own_line() {
        let own = "x[05 x 05]_C00_UltraII Filter0000.ome.tif";
        let other = "x[05 x 06]_C00_UltraII Filter0000.ome.tif";
        // the [05 x 06] tile carries [05 x 05]'s XML
        let tiles = vec![lavision_tile(
            other,
            Some(sample(own, [-1861.449219, -5300.662598])),
        )];
        let index = ConfigIndex::from_tiles(&tiles);
        let placement = place(&tiles[0], &index, None).unwrap().unwrap();
        assert_eq!(placement.source, "lavision");
        // x,-y,-z
        assert_eq!(
            placement.translation,
            Some([0.0, 5300.662598, -1270.599365])
        );
        assert!(
            placement.detail.contains("TileConfiguration line"),
            "{}",
            placement.detail
        );
        assert_eq!(placement.grid_index, Some([5, 6]));
    }

    #[test]
    fn a_tile_without_xml_is_found_in_a_sibling_configuration() {
        let own = "x[05 x 05]_C00_UltraII Filter0000.ome.tif";
        let bare = "x[04 x 05]_C00_UltraII Filter0000.ome.tif";
        let tiles = vec![
            lavision_tile(own, Some(sample(own, [-1861.449219, -5300.662598]))),
            lavision_tile(bare, None),
        ];
        let index = ConfigIndex::from_tiles(&tiles);
        let placement = place(&tiles[1], &index, None).unwrap().unwrap();
        assert_eq!(placement.translation, Some([0.0, 3393.5625, -1861.449219]));
        // without the sibling there is nothing
        let alone = ConfigIndex::from_tiles(&tiles[1..]);
        assert!(place(&tiles[1], &alone, None).is_none());
    }

    #[test]
    fn offsets_count_only_for_the_file_the_xml_names() {
        // listed nowhere, and the XML is another file's: no position
        let stray = lavision_tile("y.ome.tif", Some(sample("z.ome.tif", [1.0, 2.0])));
        let index = ConfigIndex::from_tiles(std::slice::from_ref(&stray));
        let reason = place(&stray, &index, None).unwrap().unwrap_err();
        assert!(reason.contains("another file"), "{reason}");
        // its own XML: the Offset
        let own = lavision_tile("y.ome.tif", Some(sample("y.ome.tif", [1.0, 2.0])));
        let placement = place(&own, &index, StageAxes::parse("x,y,z").ok())
            .unwrap()
            .unwrap();
        assert_eq!(placement.translation, Some([0.0, 2.0, 1.0]));
        // not part of a mosaic: an all-zero Offset is no position
        let single =
            sample("s.ome.tif", [0.0, 0.0]).replace("IsPartOfMosaic=\"1\"", "IsPartOfMosaic=\"0\"");
        let single = lavision_tile("s.ome.tif", Some(single));
        assert!(place(&single, &ConfigIndex::default(), None)
            .unwrap()
            .is_err());
        // no LaVision side at all
        assert!(place(&tile("plain.tif"), &ConfigIndex::default(), None).is_none());
    }

    #[test]
    fn the_first_ifd_description_is_read_from_classic_and_big_tiffs() {
        let dir = tempfile::tempdir().unwrap();
        let text = sample("a.ome.tif", [3.0, 4.0]);
        // classic: header, IFD at 8 with one entry pointing past it
        let mut classic = Vec::new();
        classic.extend(b"II");
        classic.extend(42u16.to_le_bytes());
        classic.extend(8u32.to_le_bytes());
        classic.extend(1u16.to_le_bytes());
        classic.extend(270u16.to_le_bytes());
        classic.extend(2u16.to_le_bytes());
        classic.extend((text.len() as u32).to_le_bytes());
        classic.extend((8u32 + 2 + 12 + 4).to_le_bytes());
        classic.extend(0u32.to_le_bytes());
        classic.extend(text.as_bytes());
        let path = dir.path().join("classic.tif");
        std::fs::write(&path, &classic).unwrap();
        assert_eq!(
            first_image_description(&path).unwrap().as_deref(),
            Some(text.as_str())
        );
        // BigTIFF, big-endian
        let mut big = Vec::new();
        big.extend(b"MM");
        big.extend(43u16.to_be_bytes());
        big.extend(8u16.to_be_bytes());
        big.extend(0u16.to_be_bytes());
        big.extend(16u64.to_be_bytes());
        big.extend(1u64.to_be_bytes());
        big.extend(270u16.to_be_bytes());
        big.extend(2u16.to_be_bytes());
        big.extend((text.len() as u64).to_be_bytes());
        big.extend((16u64 + 8 + 20 + 8).to_be_bytes());
        big.extend(0u64.to_be_bytes());
        big.extend(text.as_bytes());
        let path = dir.path().join("big.tif");
        std::fs::write(&path, &big).unwrap();
        assert_eq!(
            first_image_description(&path).unwrap().as_deref(),
            Some(text.as_str())
        );
        let path = dir.path().join("not.tif");
        std::fs::write(&path, b"hello world, not a tiff").unwrap();
        assert_eq!(first_image_description(&path).unwrap(), None);
    }
}
