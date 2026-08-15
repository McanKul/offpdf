//! Shared image gates for Edit PDF preview and export.
//!
//! Validate regular files and header dimensions *before* decoding. Preview and
//! save use the same limits so a 6000×4000 JPEG cannot enter the editor.

use crate::error::AppError;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_IMAGE_EDGE: u32 = 4_096;
pub const MAX_DOC_IMAGE_BYTES: u64 = 40 * 1024 * 1024;
pub const MAX_DOC_IMAGE_PIXELS: u64 = 32_000_000;

#[derive(Debug)]
pub struct InspectedImage {
    pub width: u32,
    pub height: u32,
    pub len: u64,
    pub bytes: Vec<u8>,
}

#[derive(Debug)]
pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
    pub source_len: u64,
}

pub fn dedupe_key(path: &str) -> String {
    Path::new(path)
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.to_string())
}

pub fn check_cancelled(cancel: Option<&AtomicBool>) -> Result<(), AppError> {
    if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
        return Err(AppError::cancelled());
    }
    Ok(())
}

fn too_large_edge() -> AppError {
    AppError::new(
        "IMAGE_TOO_LARGE",
        "Image is too large",
        format!("Images can be at most {MAX_IMAGE_EDGE} pixels on a side."),
    )
}

fn too_large_bytes() -> AppError {
    AppError::new(
        "IMAGE_TOO_LARGE",
        "Image is too large",
        "Use a PNG or JPEG smaller than 20 MB.",
    )
}

fn bad_type() -> AppError {
    AppError::new(
        "IMAGE_TYPE",
        "Unsupported image",
        "Only PNG and JPEG images can be added.",
    )
}

/// PNG IHDR width/height after the 8-byte signature.
pub fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || !bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]) {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

/// JPEG SOF width/height without decoding the scan.
pub fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 1 < bytes.len() {
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        while i < bytes.len() && bytes[i] == 0xFF {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let marker = bytes[i];
        i += 1;
        if marker == 0xD9 || marker == 0xDA {
            return None;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if i + 1 >= bytes.len() {
            return None;
        }
        let len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if len < 2 || i + len > bytes.len() {
            return None;
        }
        if matches!(
            marker,
            0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
        ) {
            if len < 7 {
                return None;
            }
            let h = u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            if w == 0 || h == 0 {
                return None;
            }
            return Some((w, h));
        }
        i += len;
    }
    None
}

pub fn inspect_image(path: &str) -> Result<InspectedImage, AppError> {
    let file = std::fs::File::open(path).map_err(|_| {
        AppError::new(
            "IMAGE_MISSING",
            "Image not found",
            "An image used in the edit could not be read.",
        )
        .with_suggestion("Choose the image again.")
    })?;
    let meta = file.metadata().map_err(|e| AppError::io("Could not read the image.", e))?;
    if !meta.file_type().is_file() {
        return Err(bad_type());
    }
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(too_large_bytes());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::io("Could not read the image.", e))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(too_large_bytes());
    }
    let dims = if bytes.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        png_dimensions(&bytes)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        jpeg_dimensions(&bytes)
    } else {
        return Err(bad_type());
    };
    let Some((width, height)) = dims else {
        return Err(AppError::new(
            "IMAGE_BAD",
            "Could not read the image",
            "The file does not look like a valid PNG or JPEG.",
        ));
    };
    if width > MAX_IMAGE_EDGE || height > MAX_IMAGE_EDGE {
        return Err(too_large_edge());
    }
    Ok(InspectedImage {
        width,
        height,
        len: bytes.len() as u64,
        bytes,
    })
}

/// Decode bytes already accepted by `inspect_image`, then re-check edges.
/// Preview and export both use this so a decoder/header mismatch cannot pass.
pub fn decode_bounded(bytes: &[u8]) -> Result<image::DynamicImage, AppError> {
    let img = image::load_from_memory(bytes).map_err(|_| {
        AppError::new(
            "IMAGE_BAD",
            "Could not read the image",
            "The file does not look like a valid PNG or JPEG.",
        )
    })?;
    if img.width() > MAX_IMAGE_EDGE || img.height() > MAX_IMAGE_EDGE {
        return Err(too_large_edge());
    }
    Ok(img)
}

pub fn load_image_rgb(path: &str) -> Result<Raster, AppError> {
    let inspected = inspect_image(path)?;
    let img = decode_bounded(&inspected.bytes)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    let mut alpha = Vec::with_capacity((w * h) as usize);
    let mut has_alpha = false;
    for px in rgba.pixels() {
        rgb.extend_from_slice(&px.0[0..3]);
        alpha.push(px.0[3]);
        if px.0[3] < 255 {
            has_alpha = true;
        }
    }
    Ok(Raster {
        w,
        h,
        rgb,
        alpha: if has_alpha { Some(alpha) } else { None },
        source_len: inspected.len,
    })
}

pub fn check_doc_budget(unique_bytes: u64, unique_pixels: u64) -> Result<(), AppError> {
    if unique_bytes > MAX_DOC_IMAGE_BYTES || unique_pixels > MAX_DOC_IMAGE_PIXELS {
        return Err(AppError::new(
            "IMAGE_TOO_LARGE",
            "Images are too large",
            "This edit uses more image data than OffPDF can save. Use fewer or smaller images.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_header_dims() {
        let mut b = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        b.extend_from_slice(&[0, 0, 0, 13, b'I', b'H', b'D', b'R']);
        b.extend_from_slice(&80u32.to_be_bytes());
        b.extend_from_slice(&60u32.to_be_bytes());
        assert_eq!(png_dimensions(&b), Some((80, 60)));
    }

    #[test]
    fn jpeg_header_dims() {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        b.extend_from_slice(&480u16.to_be_bytes());
        b.extend_from_slice(&640u16.to_be_bytes());
        b.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        assert_eq!(jpeg_dimensions(&b), Some((640, 480)));
    }

    #[test]
    fn inspect_rejects_oversized_jpeg_header_without_decode() {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-img-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.jpg");
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08];
        b.extend_from_slice(&6000u16.to_be_bytes());
        b.extend_from_slice(&4000u16.to_be_bytes());
        b.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
        std::fs::write(&path, &b).unwrap();
        let err = inspect_image(path.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "IMAGE_TOO_LARGE");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_rejects_directory() {
        let dir = std::env::temp_dir();
        let err = inspect_image(dir.to_str().unwrap()).unwrap_err();
        assert!(err.code == "IMAGE_TYPE" || err.code == "IMAGE_MISSING" || err.code == "IO_ERROR");
    }

    #[test]
    fn budget_rejects_over_cap() {
        assert!(check_doc_budget(MAX_DOC_IMAGE_BYTES + 1, 10).is_err());
        assert!(check_doc_budget(10, MAX_DOC_IMAGE_PIXELS + 1).is_err());
        assert!(check_doc_budget(10, 10).is_ok());
    }

    #[test]
    fn cancel_flag_is_honoured() {
        let flag = AtomicBool::new(true);
        assert_eq!(check_cancelled(Some(&flag)).unwrap_err().code, "CANCELLED");
        flag.store(false, Ordering::SeqCst);
        assert!(check_cancelled(Some(&flag)).is_ok());
    }

    fn unique_tmp(prefix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn inspect_rejects_oversized_len_without_reading_body() {
        let dir = unique_tmp("offpdf-img-len");
        let path = dir.join("huge.bin");
        let f = std::fs::File::create(&path).unwrap();
        f.set_len(MAX_IMAGE_BYTES + 1).unwrap();
        drop(f);
        let err = inspect_image(path.to_str().unwrap()).unwrap_err();
        assert_eq!(err.code, "IMAGE_TOO_LARGE");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inspect_and_load_small_png() {
        let dir = unique_tmp("offpdf-img-ok");
        let path = dir.join("ok.png");
        image::RgbImage::from_pixel(8, 8, image::Rgb([10, 20, 30]))
            .save(&path)
            .unwrap();
        let inspected = inspect_image(path.to_str().unwrap()).unwrap();
        assert_eq!((inspected.width, inspected.height), (8, 8));
        let decoded = decode_bounded(&inspected.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (8, 8));
        let rast = load_image_rgb(path.to_str().unwrap()).unwrap();
        assert_eq!((rast.w, rast.h), (8, 8));
        assert_eq!(rast.rgb.len(), 8 * 8 * 3);
        assert!(rast.alpha.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
