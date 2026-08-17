//! `heif-rs` — encode and decode HEIF/HEIC images via libheif.
//!
//! The libheif C library (plus its codec/support dependencies) is downloaded as a prebuilt **static** library at build
//! time and linked directly into this crate, so consumers do not need libheif installed on the host. See `build.rs`.
//!
//! Encoding uses **x265** and decoding uses **libde265** (HEVC), both statically linked.
//!
//! The API mirrors the `image` crate's codec conventions:
//! * [`HeifEncoder`] / [`HeifDecoder`] implement `image`'s `ImageEncoder` / `ImageDecoder` traits, so they plug into
//!   `DynamicImage::write_with_encoder` / `DynamicImage::from_decoder` just like the codecs bundled with `image`.
//! * a thin facade ([`encode`], [`encode_buffer`], [`decode`], [`probe`]) wraps those for one-line convenience.

mod decoder;
mod encoder;
mod error;
mod ffi;
mod info;
mod sys;

pub use decoder::{DecoderConfig, HeifDecoder};
pub use encoder::{Chroma, EncoderConfig, HeifEncoder, Preset};
pub use error::{HeifError, Result};
pub use info::{BitDepth, ImageInfo};

use std::ffi::CStr;
use std::io::Cursor;
use std::ops::Deref;

use image::{DynamicImage, EncodableLayout, ImageBuffer, ImageDecoder, PixelWithColorType};

/// Returns the version string of the linked libheif library, e.g. `"1.20.2"`.
pub fn libheif_version() -> String {
    ffi::init();
    // SAFETY: `heif_get_version` returns a pointer to a static, NUL-terminated C string.
    unsafe {
        let ptr = sys::heif_get_version();
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

/// Encode a [`DynamicImage`] to HEIC bytes using sensible defaults.
///
/// # Example
/// ```no_run
/// let img = image::open("photo.png")?;
/// let heic_bytes = heif::encode(&img)?;
/// # Ok::<(), heif::HeifError>(())
/// ```
pub fn encode(image: &DynamicImage) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    image.write_with_encoder(HeifEncoder::new(&mut buf))?;
    Ok(buf)
}

/// Encode a typed [`ImageBuffer`] directly, avoiding the runtime dispatch
/// overhead of [`DynamicImage`]. Prefer this when you already know your
/// pixel type at compile time.
///
/// # Example
/// ```no_run
/// use image::RgbaImage;
/// let img: RgbaImage = image::open("photo.png")?.into_rgba8();
/// let heic_bytes = heif::encode_buffer(&img)?;
/// # Ok::<(), heif::HeifError>(())
/// ```
pub fn encode_buffer<P, C>(buffer: &ImageBuffer<P, C>) -> Result<Vec<u8>>
where
    P: PixelWithColorType,
    [P::Subpixel]: EncodableLayout,
    C: Deref<Target = [P::Subpixel]>,
{
    let mut buf = Vec::new();
    buffer.write_with_encoder(HeifEncoder::new(&mut buf))?;
    Ok(buf)
}

/// Decode HEIC bytes into a [`DynamicImage`].
///
/// # Example
/// ```no_run
/// # let heic_bytes: Vec<u8> = Vec::new();
/// let img = heif::decode(&heic_bytes)?;
/// img.save("output.png")?;
/// # Ok::<(), heif::HeifError>(())
/// ```
pub fn decode(data: &[u8]) -> Result<DynamicImage> {
    let decoder = HeifDecoder::new(Cursor::new(data))?;
    Ok(DynamicImage::from_decoder(decoder)?)
}

/// Read only the image header — no pixel decode.
/// Useful for validation or thumbnailing pipelines.
///
/// # Example
/// ```no_run
/// # let heic_bytes: Vec<u8> = Vec::new();
/// let info = heif::probe(&heic_bytes)?;
/// println!("{}x{} @ {:?}", info.width, info.height, info.bit_depth);
/// # Ok::<(), heif::HeifError>(())
/// ```
pub fn probe(data: &[u8]) -> Result<ImageInfo> {
    let decoder = HeifDecoder::new(Cursor::new(data))?;
    let (width, height) = decoder.dimensions();
    Ok(ImageInfo {
        width,
        height,
        color_type: decoder.color_type(),
        bit_depth: decoder.bit_depth(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: calling into libheif proves the static binaries are linked and
    /// callable end-to-end.
    #[test]
    fn reports_libheif_version() {
        let version = libheif_version();
        println!("linked libheif version: {version}");
        assert!(!version.is_empty());
    }
}
