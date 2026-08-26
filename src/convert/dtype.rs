#[cfg(feature = "core-bioformats")]
use bioformats_rs::PixelType;
#[cfg(feature = "core-bioformats")]
use zarrs::array::{data_type as dt, DataType};

#[cfg(feature = "core-bioformats")]
pub fn zarr_data_type(pixel_type: PixelType) -> DataType {
    match pixel_type {
        PixelType::Bit => dt::uint8(),
        PixelType::Int8 => dt::int8(),
        PixelType::Uint8 => dt::uint8(),
        PixelType::Int16 => dt::int16(),
        PixelType::Uint16 => dt::uint16(),
        PixelType::Int32 => dt::int32(),
        PixelType::Uint32 => dt::uint32(),
        PixelType::Float32 => dt::float32(),
        PixelType::Float64 => dt::float64(),
    }
}

#[cfg(feature = "core-bioformats")]
pub fn zarr_v2_data_type(pixel_type: PixelType, little_endian: bool) -> String {
    let endian = if little_endian { '<' } else { '>' };
    match pixel_type {
        PixelType::Bit | PixelType::Uint8 => "|u1".to_string(),
        PixelType::Int8 => "|i1".to_string(),
        PixelType::Int16 => format!("{endian}i2"),
        PixelType::Uint16 => format!("{endian}u2"),
        PixelType::Int32 => format!("{endian}i4"),
        PixelType::Uint32 => format!("{endian}u4"),
        PixelType::Float32 => format!("{endian}f4"),
        PixelType::Float64 => format!("{endian}f8"),
    }
}

#[cfg(feature = "core-bioformats")]
pub fn pixel_range(pixel_type: PixelType) -> Option<(f64, f64)> {
    match pixel_type {
        PixelType::Int8 => Some((-128.0, 127.0)),
        PixelType::Uint8 | PixelType::Bit => Some((0.0, 255.0)),
        PixelType::Int16 => Some((-32768.0, 32767.0)),
        PixelType::Uint16 => Some((0.0, 65535.0)),
        PixelType::Int32 => Some((i32::MIN as f64, i32::MAX as f64)),
        PixelType::Uint32 => Some((0.0, u32::MAX as f64)),
        PixelType::Float32 | PixelType::Float64 => None,
    }
}

#[cfg(feature = "core-bioformats")]
pub fn bytes_per_sample(pixel_type: PixelType) -> usize {
    pixel_type.bytes_per_sample()
}
