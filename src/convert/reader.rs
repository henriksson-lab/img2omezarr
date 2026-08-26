use std::path::Path;

use bioformats_rs::{DimensionOrder, ImageMetadata, ImageReader, ImageReaderOptions};

pub struct BioformatsImageSource {
    reader: ImageReader,
}

impl BioformatsImageSource {
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let reader = ImageReader::open_with_options(
            path,
            ImageReaderOptions::new().flattened_resolutions(false),
        )?;
        Ok(Self { reader })
    }

    pub fn series_count(&self) -> usize {
        self.reader.series_count()
    }

    pub fn set_series(&mut self, series: usize) -> anyhow::Result<()> {
        self.reader.set_series(series)?;
        Ok(())
    }

    pub fn set_resolution(&mut self, level: usize) -> anyhow::Result<()> {
        self.reader.set_resolution(level)?;
        Ok(())
    }

    pub fn resolution_count(&self) -> usize {
        self.reader.resolution_count()
    }

    pub fn metadata(&self) -> ImageMetadata {
        self.reader.metadata().clone()
    }

    pub fn ome_metadata(&self) -> Option<bioformats_rs::OmeMetadata> {
        self.reader.ome_metadata()
    }

    pub fn open_region(
        &mut self,
        plane_index: u32,
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> anyhow::Result<Vec<u8>> {
        Ok(self.reader.open_bytes_region(plane_index, x, y, w, h)?)
    }
}

pub fn plane_index(
    order: DimensionOrder,
    z: u32,
    c: u32,
    t: u32,
    sz: u32,
    sc: u32,
    st: u32,
) -> u32 {
    match order {
        DimensionOrder::XYZCT => t * sz * sc + c * sz + z,
        DimensionOrder::XYZTC => c * sz * st + t * sz + z,
        DimensionOrder::XYCZT => t * sc * sz + z * sc + c,
        DimensionOrder::XYCTZ => z * sc * st + t * sc + c,
        DimensionOrder::XYTCZ => z * st * sc + c * st + t,
        DimensionOrder::XYTZC => c * st * sz + z * st + t,
    }
}
