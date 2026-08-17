//! Error and result types for the high-level HEIF API.

use image::ImageError;

/// High-level error type covering encode, decode, and conversion failures.
#[derive(Debug, thiserror::Error)]
pub enum HeifError {
    #[error("encoder initialization failed: {0}")]
    EncoderInit(String),
    #[error("decoder initialization failed: {0}")]
    DecoderInit(String),
    #[error("encode failed: {0}")]
    Encode(String),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("invalid image dimensions: {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error(transparent)]
    Image(#[from] ImageError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Convenience alias for results produced by this crate.
pub type Result<T> = std::result::Result<T, HeifError>;
