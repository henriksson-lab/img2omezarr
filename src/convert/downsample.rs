#[cfg(feature = "core-bioformats")]
use crate::convert::config::Downsampling;

#[cfg(feature = "core-bioformats")]
use bioformats_rs::PixelType;

pub fn downsample_nearest_bytes(
    input: &[u8],
    width: usize,
    height: usize,
    bytes_per_sample: usize,
    factor: usize,
) -> Vec<u8> {
    let out_w = (width / factor).max(1);
    let out_h = (height / factor).max(1);
    let mut out = vec![0u8; out_w * out_h * bytes_per_sample];
    for y in 0..out_h {
        for x in 0..out_w {
            let src = ((y * factor) * width + (x * factor)) * bytes_per_sample;
            let dst = (y * out_w + x) * bytes_per_sample;
            out[dst..dst + bytes_per_sample].copy_from_slice(&input[src..src + bytes_per_sample]);
        }
    }
    out
}

#[cfg(feature = "core-bioformats")]
pub fn downsample_bytes(
    input: &[u8],
    width: usize,
    height: usize,
    pixel_type: PixelType,
    little_endian: bool,
    factor: usize,
    method: Downsampling,
) -> Vec<u8> {
    match method {
        Downsampling::Nearest => {
            downsample_nearest_bytes(input, width, height, pixel_type.bytes_per_sample(), factor)
        }
        Downsampling::Average => {
            downsample_average_bytes(input, width, height, pixel_type, little_endian, factor)
        }
    }
}

#[cfg(feature = "core-bioformats")]
fn downsample_average_bytes(
    input: &[u8],
    width: usize,
    height: usize,
    pixel_type: PixelType,
    little_endian: bool,
    factor: usize,
) -> Vec<u8> {
    match pixel_type {
        PixelType::Bit | PixelType::Uint8 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| bytes[0] as f64,
            |value| vec![value.round().clamp(0.0, u8::MAX as f64) as u8],
        ),
        PixelType::Int8 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| bytes[0] as i8 as f64,
            |value| vec![(value.round().clamp(i8::MIN as f64, i8::MAX as f64) as i8) as u8],
        ),
        PixelType::Int16 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| i16::from_ne_bytes(endian2(bytes, little_endian)) as f64,
            |value| {
                encode_i16(
                    value.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16,
                    little_endian,
                )
            },
        ),
        PixelType::Uint16 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| u16::from_ne_bytes(endian2(bytes, little_endian)) as f64,
            |value| {
                encode_u16(
                    value.round().clamp(0.0, u16::MAX as f64) as u16,
                    little_endian,
                )
            },
        ),
        PixelType::Int32 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| i32::from_ne_bytes(endian4(bytes, little_endian)) as f64,
            |value| {
                encode_i32(
                    value.round().clamp(i32::MIN as f64, i32::MAX as f64) as i32,
                    little_endian,
                )
            },
        ),
        PixelType::Uint32 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| u32::from_ne_bytes(endian4(bytes, little_endian)) as f64,
            |value| {
                encode_u32(
                    value.round().clamp(0.0, u32::MAX as f64) as u32,
                    little_endian,
                )
            },
        ),
        PixelType::Float32 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| f32::from_ne_bytes(endian4(bytes, little_endian)) as f64,
            |value| encode_f32(value as f32, little_endian),
        ),
        PixelType::Float64 => average_numeric(
            input,
            width,
            height,
            factor,
            |bytes| f64::from_ne_bytes(endian8(bytes, little_endian)),
            |value| encode_f64(value, little_endian),
        ),
    }
}

#[cfg(feature = "core-bioformats")]
fn average_numeric<F, G>(
    input: &[u8],
    width: usize,
    height: usize,
    factor: usize,
    decode: F,
    encode: G,
) -> Vec<u8>
where
    F: Fn(&[u8]) -> f64,
    G: Fn(f64) -> Vec<u8>,
{
    let bytes_per_sample = input.len() / (width * height).max(1);
    let out_w = (width / factor).max(1);
    let out_h = (height / factor).max(1);
    let mut out = Vec::with_capacity(out_w * out_h * bytes_per_sample);
    for y in 0..out_h {
        for x in 0..out_w {
            let mut sum = 0.0;
            let mut count = 0usize;
            for by in y * factor..((y + 1) * factor).min(height) {
                for bx in x * factor..((x + 1) * factor).min(width) {
                    let src = (by * width + bx) * bytes_per_sample;
                    sum += decode(&input[src..src + bytes_per_sample]);
                    count += 1;
                }
            }
            out.extend(encode(sum / count.max(1) as f64));
        }
    }
    out
}

#[cfg(feature = "core-bioformats")]
fn endian2(bytes: &[u8], little_endian: bool) -> [u8; 2] {
    let value = [bytes[0], bytes[1]];
    if little_endian == cfg!(target_endian = "little") {
        value
    } else {
        [value[1], value[0]]
    }
}

#[cfg(feature = "core-bioformats")]
fn endian4(bytes: &[u8], little_endian: bool) -> [u8; 4] {
    let value = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if little_endian == cfg!(target_endian = "little") {
        value
    } else {
        [value[3], value[2], value[1], value[0]]
    }
}

#[cfg(feature = "core-bioformats")]
fn endian8(bytes: &[u8], little_endian: bool) -> [u8; 8] {
    let value = [
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ];
    if little_endian == cfg!(target_endian = "little") {
        value
    } else {
        [
            value[7], value[6], value[5], value[4], value[3], value[2], value[1], value[0],
        ]
    }
}

#[cfg(feature = "core-bioformats")]
fn encode_i16(value: i16, little_endian: bool) -> Vec<u8> {
    if little_endian {
        value.to_le_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

#[cfg(feature = "core-bioformats")]
fn encode_u16(value: u16, little_endian: bool) -> Vec<u8> {
    if little_endian {
        value.to_le_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

#[cfg(feature = "core-bioformats")]
fn encode_i32(value: i32, little_endian: bool) -> Vec<u8> {
    if little_endian {
        value.to_le_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

#[cfg(feature = "core-bioformats")]
fn encode_u32(value: u32, little_endian: bool) -> Vec<u8> {
    if little_endian {
        value.to_le_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

#[cfg(feature = "core-bioformats")]
fn encode_f32(value: f32, little_endian: bool) -> Vec<u8> {
    if little_endian {
        value.to_le_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

#[cfg(feature = "core-bioformats")]
fn encode_f64(value: f64, little_endian: bool) -> Vec<u8> {
    if little_endian {
        value.to_le_bytes().to_vec()
    } else {
        value.to_be_bytes().to_vec()
    }
}

#[cfg(all(test, feature = "core-bioformats"))]
mod tests {
    use super::*;

    #[test]
    fn nearest_downsamples_u8() {
        let input: Vec<u8> = (0..16).collect();
        assert_eq!(
            downsample_bytes(
                &input,
                4,
                4,
                PixelType::Uint8,
                true,
                2,
                Downsampling::Nearest
            ),
            vec![0, 2, 8, 10]
        );
    }

    #[test]
    fn average_downsamples_u8() {
        let input: Vec<u8> = (0..16).collect();
        assert_eq!(
            downsample_bytes(
                &input,
                4,
                4,
                PixelType::Uint8,
                true,
                2,
                Downsampling::Average
            ),
            vec![3, 5, 11, 13]
        );
    }

    #[test]
    fn average_downsamples_u16_big_endian() {
        let input = [2u16, 4, 10, 12]
            .into_iter()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<_>>();
        assert_eq!(
            downsample_bytes(
                &input,
                2,
                2,
                PixelType::Uint16,
                false,
                2,
                Downsampling::Average
            ),
            7u16.to_be_bytes().to_vec()
        );
    }
}
