//! `set-positions`: write positions into existing OME-Zarr images by
//! rewriting only each image group's metadata file, `zarr.json` (NGFF 0.5,
//! attributes under `attributes.ome`) or `.zattrs` (NGFF 0.4). Arrays and
//! chunks are not touched.
//!
//! The level-0 translation is set, and every level's translation moves by
//! the same amount, so whatever relation the levels had (none, or the
//! half-voxel offsets of a downsampled pyramid) is kept.

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::lavision::{self, LaVisionSource, LaVisionXml};
use super::{
    file_name, grid, name_stem, place_batch_with, Originals, Placement, PositionSettings, Tile,
};

/// `set-positions`: place every image under `paths` as one batch, check the
/// layout, then rewrite each image's group metadata (unless `dry_run`).
/// Every document is built before the first is written, so an image that
/// cannot take its position leaves all of them as they were.
pub fn set_positions(
    paths: &[PathBuf],
    settings: &PositionSettings,
    dry_run: bool,
    warn: &dyn Fn(&str),
) -> anyhow::Result<Vec<(ZarrImage, Placement)>> {
    if !settings.user_source() && settings.originals.is_none() {
        anyhow::bail!(
            "set-positions needs --positions FILE, --overlap/--pitch with --grid-unit, or \
             --from-originals DIR"
        );
    }
    let mut images = Vec::new();
    for path in paths {
        images.extend(find_images(path)?);
    }
    let configurations = match &settings.originals {
        Some(originals) if !settings.user_source() => attach_originals(&mut images, originals)?,
        _ => lavision::ConfigIndex::default(),
    };
    let tiles: Vec<Tile> = images.iter().map(|image| image.tile.clone()).collect();
    let placements = place_batch_with(&tiles, settings, &configurations, warn)?;
    let documents = images
        .iter()
        .zip(&placements)
        .map(|(image, placement)| with_placement(image, placement))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if !dry_run {
        for (image, document) in images.iter().zip(&documents) {
            write_metadata(image, document)?;
        }
    }
    Ok(images.into_iter().zip(placements).collect())
}

/// `--from-originals`: give every image the LaVision side of the original
/// file it was converted from, and return the tile configurations of every
/// file in the directory (a tile's file may carry no XML; another file's
/// configuration lists it).
///
/// An image's original is the file whose name without extension is the
/// image's, else the files with the image's grid index (the default
/// `<row> x <column>` pattern on both names). Several files with one grid
/// index (the channels of a tile, a renamed copy) are fine when every one
/// that gives a position gives the same; otherwise the match is refused.
/// Only each file's first TIFF directory is read.
pub fn attach_originals(
    images: &mut [ZarrImage],
    originals: &Originals,
) -> anyhow::Result<lavision::ConfigIndex> {
    let mut files: Vec<(String, PathBuf)> = std::fs::read_dir(&originals.dir)
        .map_err(|error| anyhow::anyhow!("listing {}: {error}", originals.dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_file())
        .map(|path| (file_name(&path), path))
        .filter(|(name, _)| {
            originals
                .matching
                .as_ref()
                .is_none_or(|pattern| pattern.is_match(name))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        anyhow::bail!(
            "--from-originals {}: no files{}",
            originals.dir.display(),
            if originals.matching.is_some() {
                " match --originals-match"
            } else {
                ""
            }
        );
    }
    let mut xml: Vec<Option<LaVisionXml>> = Vec::with_capacity(files.len());
    let mut index = lavision::ConfigIndex::default();
    for (name, path) in &files {
        let parsed = lavision::first_image_description(path)?
            .as_deref()
            .and_then(lavision::parse);
        if let Some(parsed) = &parsed {
            index.add(parsed, name);
        }
        xml.push(parsed);
    }
    for image in images.iter_mut() {
        let name = &image.tile.name;
        let stem = name_stem(name);
        let mut candidates: Vec<usize> = (0..files.len())
            .filter(|&i| name_stem(&files[i].0) == stem)
            .collect();
        if candidates.is_empty() {
            if let Some(cell) = grid::default_index(name) {
                candidates = (0..files.len())
                    .filter(|&i| grid::default_index(&files[i].0) == Some(cell))
                    .collect();
            }
        }
        if candidates.is_empty() {
            anyhow::bail!(
                "--from-originals: no file under {} matches {name} (by its name without \
                 extension, or by its grid index)",
                originals.dir.display()
            );
        }
        let source = |i: usize| LaVisionSource {
            file_name: files[i].0.clone(),
            xml: xml[i].clone(),
        };
        let positioned: Vec<(usize, lavision::StagePosition)> = candidates
            .iter()
            .filter_map(|&i| {
                lavision::stage_position(&source(i), &index)
                    .ok()
                    .map(|position| (i, position))
            })
            .collect();
        if let Some((_, first)) = positioned.first() {
            if let Some((other, _)) = positioned
                .iter()
                .find(|(_, position)| position.xyz != first.xyz)
            {
                anyhow::bail!(
                    "--from-originals: {name} matches {} and {}, which give different \
                     positions; narrow the files with --originals-match",
                    files[positioned[0].0].0,
                    files[*other].0
                );
            }
        }
        let chosen = positioned.first().map_or(candidates[0], |(i, _)| *i);
        image.tile.lavision = Some(source(chosen));
    }
    Ok(index)
}

/// One image group found under a path given to `set-positions`.
#[derive(Debug, Clone)]
pub struct ZarrImage {
    pub tile: Tile,
    /// The image group's directory.
    pub group: PathBuf,
    /// Its metadata file: `zarr.json` or `.zattrs`.
    pub metadata: PathBuf,
    pub v05: bool,
}

/// The images a path holds: the path itself when it is a multiscale image,
/// or each series of a bioformats2raw collection.
pub fn find_images(path: &Path) -> anyhow::Result<Vec<ZarrImage>> {
    let (metadata, v05) = metadata_file(path)?;
    let document = read_json(&metadata)?;
    let attrs = attributes(&document, v05);
    if multiscale(attrs).is_some() {
        return Ok(vec![open_image(path, path, None)?]);
    }
    let layout = if v05 {
        attrs.pointer("/ome/bioformats2raw.layout")
    } else {
        attrs.get("bioformats2raw.layout")
    };
    if layout.is_none() {
        anyhow::bail!(
            "{}: neither a multiscale image nor a bioformats2raw collection",
            path.display()
        );
    }
    let series = series_paths(path, v05)?;
    series
        .iter()
        .enumerate()
        .map(|(index, child)| open_image(path, &path.join(child), Some(index)))
        .collect()
}

/// The collection's series paths: `OME`'s `series` list, else `0`, `1`, ...
fn series_paths(path: &Path, v05: bool) -> anyhow::Result<Vec<String>> {
    if let Ok((metadata, _)) = metadata_file(&path.join("OME")) {
        let document = read_json(&metadata)?;
        let attrs = attributes(&document, v05);
        let list = attrs.pointer("/ome/series").or_else(|| attrs.get("series"));
        if let Some(list) = list.and_then(Value::as_array) {
            return Ok(list
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect());
        }
    }
    let mut paths = Vec::new();
    while path.join(paths.len().to_string()).is_dir() {
        paths.push(paths.len().to_string());
    }
    Ok(paths)
}

fn metadata_file(group: &Path) -> anyhow::Result<(PathBuf, bool)> {
    if group.join("zarr.json").is_file() {
        Ok((group.join("zarr.json"), true))
    } else if group.join(".zattrs").is_file() {
        Ok((group.join(".zattrs"), false))
    } else {
        anyhow::bail!("{}: no zarr.json or .zattrs", group.display())
    }
}

fn read_json(path: &Path) -> anyhow::Result<Value> {
    let bytes = std::fs::read(path)
        .map_err(|error| anyhow::anyhow!("reading {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| anyhow::anyhow!("parsing {}: {error}", path.display()))
}

/// The group attributes: `attributes` in a `zarr.json`, the whole `.zattrs`.
fn attributes(document: &Value, v05: bool) -> &Value {
    if v05 {
        document.get("attributes").unwrap_or(&Value::Null)
    } else {
        document
    }
}

fn attributes_mut(document: &mut Value, v05: bool) -> anyhow::Result<&mut Map<String, Value>> {
    let attrs = if v05 {
        document
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("zarr.json is not an object"))?
            .entry("attributes")
            .or_insert_with(|| json!({}))
    } else {
        document
    };
    attrs
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("group attributes are not an object"))
}

fn multiscale(attrs: &Value) -> Option<&Value> {
    attrs
        .pointer("/ome/multiscales/0")
        .or_else(|| attrs.pointer("/multiscales/0"))
}

fn multiscale_mut(attrs: &mut Map<String, Value>) -> Option<&mut Value> {
    if attrs
        .get("ome")
        .and_then(|ome| ome.get("multiscales"))
        .is_some()
    {
        attrs.get_mut("ome")?.pointer_mut("/multiscales/0")
    } else {
        attrs.get_mut("multiscales")?.get_mut(0)
    }
}

/// The multiscale's axis count, and its spatial axes' positions and units,
/// `(z, y, x)`.
type SpatialAxes = (usize, [Option<usize>; 3], [Option<String>; 3]);

fn spatial_axes(multiscale: &Value) -> anyhow::Result<SpatialAxes> {
    let axes = multiscale["axes"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("the multiscale has no axes"))?;
    let mut index = [None; 3];
    let mut units = [None, None, None];
    for (position, axis) in axes.iter().enumerate() {
        // NGFF 0.3 axes are plain names
        let name = axis
            .get("name")
            .or(Some(axis))
            .and_then(Value::as_str)
            .unwrap_or("");
        if let Some(slot) = ["z", "y", "x"]
            .iter()
            .position(|candidate| *candidate == name)
        {
            index[slot] = Some(position);
            units[slot] = axis.get("unit").and_then(Value::as_str).map(str::to_string);
        }
    }
    if index[1].is_none() || index[2].is_none() {
        anyhow::bail!("the multiscale has no y and x axes");
    }
    Ok((axes.len(), index, units))
}

fn transform<'a>(transforms: &'a Value, kind: &str) -> Option<&'a Vec<Value>> {
    transforms
        .as_array()?
        .iter()
        .find(|row| row["type"] == kind)?
        .get(kind)?
        .as_array()
}

fn open_image(root: &Path, group: &Path, series: Option<usize>) -> anyhow::Result<ZarrImage> {
    let (metadata, v05) = metadata_file(group)?;
    let document = read_json(&metadata)?;
    let attrs = attributes(&document, v05);
    let multiscale = multiscale(attrs)
        .ok_or_else(|| anyhow::anyhow!("{}: not a multiscale image", group.display()))?;
    let (count, index, units) = spatial_axes(multiscale)
        .map_err(|error| anyhow::anyhow!("{}: {error}", group.display()))?;
    let level0 = &multiscale["datasets"][0];
    let path0 = level0["path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("{}: the multiscale has no datasets", group.display()))?;
    let scale = transform(&level0["coordinateTransformations"], "scale")
        .filter(|scale| scale.len() == count)
        .ok_or_else(|| {
            anyhow::anyhow!("{}: level 0 has no scale over every axis", group.display())
        })?;
    let array = group.join(path0);
    let array_meta = if array.join("zarr.json").is_file() {
        array.join("zarr.json")
    } else {
        array.join(".zarray")
    };
    let shape = read_json(&array_meta)?["shape"]
        .as_array()
        .filter(|shape| shape.len() == count)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("{}: no shape over every axis", array_meta.display()))?;
    let pick = |values: &[Value], slot: usize, missing: f64| {
        index[slot]
            .and_then(|position| values[position].as_f64())
            .unwrap_or(missing)
    };
    Ok(ZarrImage {
        tile: Tile {
            name: file_name(root),
            path: root.to_path_buf(),
            series,
            shape: [0, 1, 2].map(|slot| pick(&shape, slot, 1.0) as u64),
            scale: [0, 1, 2].map(|slot| pick(scale, slot, 1.0)),
            units,
            stage: None,
            lavision: None,
        },
        group: group.to_path_buf(),
        metadata,
        v05,
    })
}

/// The image's metadata with `placement` written: the level-0 translation
/// (when there is one) and the provenance attribute.
pub fn with_placement(image: &ZarrImage, placement: &Placement) -> anyhow::Result<Value> {
    let mut document = read_json(&image.metadata)?;
    let attrs = attributes_mut(&mut document, image.v05)?;
    let label = image.group.display().to_string();
    let multiscale =
        multiscale_mut(attrs).ok_or_else(|| anyhow::anyhow!("{label}: not a multiscale image"))?;
    if let Some(global) = multiscale.get("coordinateTransformations") {
        if global.as_array().is_some_and(|rows| !rows.is_empty()) {
            anyhow::bail!(
                "{label}: multiscale-level coordinateTransformations are not supported; \
                 the position belongs in level 0's"
            );
        }
    }
    let (count, index, _) = spatial_axes(multiscale)?;
    if let Some(target) = placement.translation {
        let datasets = multiscale["datasets"]
            .as_array_mut()
            .ok_or_else(|| anyhow::anyhow!("{label}: no datasets"))?;
        let old = transform(&datasets[0]["coordinateTransformations"], "translation")
            .map(|values| {
                values
                    .iter()
                    .map(|value| value.as_f64().unwrap_or(0.0))
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0; count]);
        if old.len() != count {
            anyhow::bail!("{label}: level 0's translation does not cover every axis");
        }
        let mut delta = vec![0.0; count];
        for slot in 0..3 {
            if let Some(position) = index[slot] {
                delta[position] = target[slot] - old[position];
            }
        }
        for dataset in datasets {
            let transforms = dataset["coordinateTransformations"]
                .as_array_mut()
                .ok_or_else(|| {
                    anyhow::anyhow!("{label}: a level without coordinateTransformations")
                })?;
            match transforms
                .iter_mut()
                .find(|row| row["type"] == "translation")
            {
                Some(row) => {
                    let values = row["translation"]
                        .as_array_mut()
                        .filter(|values| values.len() == count)
                        .ok_or_else(|| {
                            anyhow::anyhow!("{label}: a translation that does not cover every axis")
                        })?;
                    for (value, delta) in values.iter_mut().zip(&delta) {
                        *value = json!(value.as_f64().unwrap_or(0.0) + delta);
                    }
                }
                None => {
                    // after the scale, as NGFF orders them
                    let at = transforms
                        .iter()
                        .position(|row| row["type"] == "scale")
                        .map_or(transforms.len(), |position| position + 1);
                    transforms.insert(at, json!({ "type": "translation", "translation": delta }));
                }
            }
        }
    }
    let ours = attrs.entry("img2omezarr").or_insert_with(|| json!({}));
    if !ours.is_object() {
        *ours = json!({});
    }
    ours["positions"] = placement.provenance();
    Ok(document)
}

/// Write the image's metadata file: a temporary file beside it, then a
/// rename, so a reader never sees half a file. (A metadata file that was a
/// symlink becomes a file; what it pointed to is left alone.)
pub fn write_metadata(image: &ZarrImage, document: &Value) -> anyhow::Result<()> {
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    let temp = image.metadata.with_extension("json.img2omezarr-tmp");
    std::fs::write(&temp, text)
        .map_err(|error| anyhow::anyhow!("writing {}: {error}", temp.display()))?;
    std::fs::rename(&temp, &image.metadata)
        .map_err(|error| anyhow::anyhow!("replacing {}: {error}", image.metadata.display()))?;
    Ok(())
}
