//! Image metadata types returned by probing and decoding.

use image::ColorType;

/// Metadata describing a HEIF image, without (necessarily) decoding its pixels.
#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
    /// Reuse `image`'s enum — no point rolling our own.
    pub color_type: ColorType,
    /// Bit depth per channel; `image` has no equivalent for this.
    pub bit_depth: BitDepth,
}

/// Bits per channel of a HEIF image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BitDepth {
    Eight,
    Ten,
    Twelve,
}
