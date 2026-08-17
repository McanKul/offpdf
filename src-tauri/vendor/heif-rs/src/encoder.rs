//! HEIF encoder, mirroring the `image` crate's per-format encoder convention.
//!
//! [`HeifEncoder`] is generic over a [`Write`] sink and implements [`ImageEncoder`], so it slots into
//! `DynamicImage::write_with_encoder` exactly like the codecs that ship with the `image` crate (e.g. `JpegEncoder`,
//! `WebPEncoder`). Encoding uses x265 (HEVC) under the hood.

use std::ffi::{CString, c_void};
use std::io::Write;
use std::ptr;

use image::error::{EncodingError, ImageFormatHint, UnsupportedError, UnsupportedErrorKind};
use image::{ExtendedColorType, ImageEncoder, ImageError, ImageResult};

use crate::error::HeifError;
use crate::ffi;
use crate::info::BitDepth;
use crate::sys;

/// x265 speed/efficiency preset — the native "speed" control. Faster presets encode quicker at the cost of compression
/// efficiency. Maps 1:1 to libheif's `preset` param.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Ultrafast,
    Superfast,
    Veryfast,
    Faster,
    Fast,
    Medium,
    Slow,
    Slower,
    Veryslow,
    Placebo,
}

impl Preset {
    fn as_str(self) -> &'static str {
        match self {
            Preset::Ultrafast => "ultrafast",
            Preset::Superfast => "superfast",
            Preset::Veryfast => "veryfast",
            Preset::Faster => "faster",
            Preset::Fast => "fast",
            Preset::Medium => "medium",
            Preset::Slow => "slow",
            Preset::Slower => "slower",
            Preset::Veryslow => "veryslow",
            Preset::Placebo => "placebo",
        }
    }
}

/// Chroma subsampling of the encoded HEVC stream. Maps to libheif's `chroma` param.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chroma {
    /// 4:2:0 — smallest, the usual choice for photographs.
    Yuv420,
    /// 4:2:2.
    Yuv422,
    /// 4:4:4 — no chroma subsampling.
    Yuv444,
}

impl Chroma {
    fn as_str(self) -> &'static str {
        match self {
            Chroma::Yuv420 => "420",
            Chroma::Yuv422 => "422",
            Chroma::Yuv444 => "444",
        }
    }
}

/// Tunable parameters for the x265 encoder.
///
/// Field names, ranges, and defaults mirror libheif's x265 plugin parameters.
pub struct EncoderConfig {
    /// Quality, range 0–100 (higher = better); default 60. Ignored when `lossless` is set.
    /// Maps to `heif_encoder_set_lossy_quality`.
    pub quality: u8,
    /// Lossless encoding; default `false`. When `true`, `quality` is ignored.
    /// Maps to `heif_encoder_set_lossless`.
    pub lossless: bool,
    /// x265 speed preset; default [`Preset::Slow`] (libheif's native default).
    pub preset: Preset,
    /// Chroma subsampling; default [`Chroma::Yuv420`] (libheif's native default).
    pub chroma: Chroma,
    /// Output bit depth, default [`BitDepth::Eight`].
    ///
    /// libheif supports 8/10/12-bit natively, but the prebuilt x265 in the bundled binaries is an 8-bit build;
    /// requesting 10/12-bit there makes libheif return an error, which is surfaced as [`HeifError::Encode`].
    pub bit_depth: BitDepth,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            quality: 60,
            lossless: false,
            preset: Preset::Slow,
            chroma: Chroma::Yuv420,
            bit_depth: BitDepth::Eight,
        }
    }
}

/// HEIF encoder writing to `W`, using x265.
///
/// # Example
/// ```no_run
/// use heif::HeifEncoder;
/// use image::ImageEncoder;
///
/// let img = image::open("photo.png")?;
/// let mut buf = Vec::new();
/// img.write_with_encoder(HeifEncoder::new(&mut buf))?;
/// # Ok::<(), image::ImageError>(())
/// ```
pub struct HeifEncoder<W: Write> {
    writer: W,
    config: EncoderConfig,
}

impl<W: Write> HeifEncoder<W> {
    /// Create an encoder writing to `w` with default settings.
    pub fn new(w: W) -> Self {
        Self {
            writer: w,
            config: EncoderConfig::default(),
        }
    }

    /// Create an encoder writing to `w` with an explicit configuration.
    pub fn new_with_config(w: W, config: EncoderConfig) -> Self {
        Self { writer: w, config }
    }

    pub fn with_quality(mut self, quality: u8) -> Self {
        self.config.quality = quality;
        self
    }

    pub fn with_lossless(mut self, lossless: bool) -> Self {
        self.config.lossless = lossless;
        self
    }

    pub fn with_preset(mut self, preset: Preset) -> Self {
        self.config.preset = preset;
        self
    }

    pub fn with_chroma(mut self, chroma: Chroma) -> Self {
        self.config.chroma = chroma;
        self
    }

    pub fn with_bit_depth(mut self, bit_depth: BitDepth) -> Self {
        self.config.bit_depth = bit_depth;
        self
    }
}

/// How an input pixel buffer maps onto the interleaved RGB image libheif consumes.
struct Layout {
    /// Channels per pixel in the *input* `buf` (1=L, 2=La, 3=Rgb, 4=Rgba).
    src_channels: usize,
    /// Bytes per *input* sample (1 for 8-bit, 2 for 16-bit, native-endian).
    src_sample_bytes: usize,
    /// Channels in the interleaved image handed to libheif (3=RGB, 4=RGBA).
    out_channels: usize,
    /// Whether the input is grayscale and must be expanded to RGB/RGBA.
    gray: bool,
    /// Whether the input carries an alpha channel.
    alpha: bool,
}

/// Maps a supported [`ExtendedColorType`] to its [`Layout`], or `None` if unsupported.
fn layout_for(color_type: ExtendedColorType) -> Option<Layout> {
    use ExtendedColorType as E;

    let l = |src_channels, src_sample_bytes, out_channels, gray, alpha| {
        Some(Layout {
            src_channels,
            src_sample_bytes,
            out_channels,
            gray,
            alpha,
        })
    };

    match color_type {
        E::L8 => l(1, 1, 3, true, false),
        E::La8 => l(2, 1, 4, true, true),
        E::Rgb8 => l(3, 1, 3, false, false),
        E::Rgba8 => l(4, 1, 4, false, true),
        E::L16 => l(1, 2, 3, true, false),
        E::La16 => l(2, 2, 4, true, true),
        E::Rgb16 => l(3, 2, 3, false, false),
        E::Rgba16 => l(4, 2, 4, false, true),
        _ => None,
    }
}

fn image_depth(bit_depth: BitDepth) -> u32 {
    match bit_depth {
        BitDepth::Eight => 8,
        BitDepth::Ten => 10,
        BitDepth::Twelve => 12,
    }
}

/// Reads one native-endian input sample at byte offset `off`.
#[inline]
fn read_sample(buf: &[u8], off: usize, sample_bytes: usize) -> u32 {
    if sample_bytes == 1 {
        buf[off] as u32
    } else {
        u16::from_ne_bytes([buf[off], buf[off + 1]]) as u32
    }
}

/// Rescales a sample value from `src_bits` to `dst_bits` (libheif does not auto-scale).
#[inline]
fn scale(value: u32, src_bits: u32, dst_bits: u32) -> u32 {
    if dst_bits >= src_bits {
        value << (dst_bits - src_bits)
    } else {
        value >> (src_bits - dst_bits)
    }
}

impl EncoderConfig {
    /// Returns the libheif chroma format of the interleaved plane for the given alpha presence and output bit depth.
    fn interleaved_chroma(&self, alpha: bool) -> sys::heif_chroma {
        match (image_depth(self.bit_depth) > 8, alpha) {
            (false, false) => sys::heif_chroma_heif_chroma_interleaved_RGB,
            (false, true) => sys::heif_chroma_heif_chroma_interleaved_RGBA,
            (true, false) => sys::heif_chroma_heif_chroma_interleaved_RRGGBB_LE,
            (true, true) => sys::heif_chroma_heif_chroma_interleaved_RRGGBBAA_LE,
        }
    }

    /// Runs the full libheif encode pipeline, returning the encoded HEIC bytes.
    fn encode(
        &self,
        buf: &[u8],
        width: u32,
        height: u32,
        color_type: ExtendedColorType,
    ) -> Result<Vec<u8>, EncodeError> {
        if width == 0 || height == 0 {
            return Err(EncodeError::Heif(HeifError::InvalidDimensions { width, height }));
        }

        let layout = layout_for(color_type).ok_or(EncodeError::Unsupported(color_type))?;

        let expected = width as usize * height as usize * layout.src_channels * layout.src_sample_bytes;
        if buf.len() != expected {
            return Err(EncodeError::Heif(HeifError::Encode(format!(
                "buffer length {} does not match {width}x{height} with {} channels of {} byte(s)",
                buf.len(),
                layout.src_channels,
                layout.src_sample_bytes,
            ))));
        }

        ffi::init();

        let chroma = self.interleaved_chroma(layout.alpha);

        // SAFETY: every raw pointer below is checked, and the corresponding libheif object is freed on all paths before
        // returning.
        unsafe {
            let mut image: *mut sys::heif_image = ptr::null_mut();
            ffi::check(sys::heif_image_create(
                width as i32,
                height as i32,
                sys::heif_colorspace_heif_colorspace_RGB,
                chroma,
                &mut image,
            ))
            .map_err(|m| EncodeError::Heif(HeifError::EncoderInit(m)))?;

            let result = self.encode_with_image(image, buf, &layout, width, height);
            sys::heif_image_release(image);
            result
        }
    }

    /// Inner half of [`encode`](Self::encode): assumes `image` is a valid, owned `heif_image` (freed by the caller) and
    /// produces the encoded bytes.
    ///
    /// # Safety
    /// `image` must be a non-null pointer from `heif_image_create`, and `buf` must describe `width`×`height` pixels
    /// laid out per `layout`.
    unsafe fn encode_with_image(
        &self,
        image: *mut sys::heif_image,
        buf: &[u8],
        layout: &Layout,
        width: u32,
        height: u32,
    ) -> Result<Vec<u8>, EncodeError> {
        let out_depth = image_depth(self.bit_depth);
        let out_sample_bytes = if out_depth > 8 { 2 } else { 1 };

        // SAFETY: see this function's contract; all libheif handles are checked and freed.
        unsafe {
            ffi::check(sys::heif_image_add_plane(
                image,
                sys::heif_channel_heif_channel_interleaved,
                width as i32,
                height as i32,
                out_depth as i32,
            ))
            .map_err(|m| EncodeError::Heif(HeifError::EncoderInit(m)))?;

            // Fill the interleaved plane row by row, honoring libheif's stride and scaling samples to the output bit
            // depth.
            let mut stride: i32 = 0;
            let plane = sys::heif_image_get_plane(image, sys::heif_channel_heif_channel_interleaved, &mut stride);
            if plane.is_null() {
                return Err(EncodeError::Heif(HeifError::EncoderInit(
                    "heif_image_get_plane returned null".into(),
                )));
            }
            self.fill_plane(
                plane,
                stride as usize,
                buf,
                layout,
                width,
                height,
                out_depth,
                out_sample_bytes,
            );

            // Context + encoder.
            let ctx = sys::heif_context_alloc();
            if ctx.is_null() {
                return Err(EncodeError::Heif(HeifError::EncoderInit(
                    "heif_context_alloc returned null".into(),
                )));
            }

            let result = self.encode_into_context(ctx, image);
            sys::heif_context_free(ctx);
            result
        }
    }

    /// Acquires the encoder, applies the config, encodes `image`, and writes the result.
    ///
    /// # Safety
    /// `ctx` and `image` must be valid libheif handles owned by the caller.
    unsafe fn encode_into_context(
        &self,
        ctx: *mut sys::heif_context,
        image: *mut sys::heif_image,
    ) -> Result<Vec<u8>, EncodeError> {
        // SAFETY: see contract; the encoder/handle/options are freed before returning.
        unsafe {
            let mut encoder: *mut sys::heif_encoder = ptr::null_mut();
            ffi::check(sys::heif_context_get_encoder_for_format(
                ctx,
                sys::heif_compression_format_heif_compression_HEVC,
                &mut encoder,
            ))
            .map_err(|m| EncodeError::Heif(HeifError::EncoderInit(m)))?;

            let result = self.encode_with_encoder(ctx, image, encoder);
            sys::heif_encoder_release(encoder);
            result
        }
    }

    /// Sets parameters on `encoder`, encodes, and serializes to bytes.
    ///
    /// # Safety
    /// `ctx`, `image`, and `encoder` must be valid libheif handles owned by the caller.
    unsafe fn encode_with_encoder(
        &self,
        ctx: *mut sys::heif_context,
        image: *mut sys::heif_image,
        encoder: *mut sys::heif_encoder,
    ) -> Result<Vec<u8>, EncodeError> {
        // SAFETY: see contract.
        unsafe {
            if self.lossless {
                ffi::check(sys::heif_encoder_set_lossless(encoder, 1))
                    .map_err(|m| EncodeError::Heif(HeifError::Encode(m)))?;
            } else {
                ffi::check(sys::heif_encoder_set_lossy_quality(encoder, self.quality as i32))
                    .map_err(|m| EncodeError::Heif(HeifError::Encode(m)))?;
            }

            set_string_param(encoder, "preset", self.preset.as_str())?;
            set_string_param(encoder, "chroma", self.chroma.as_str())?;

            // Encode into a (discarded) image handle.
            let options = sys::heif_encoding_options_alloc();
            let mut handle: *mut sys::heif_image_handle = ptr::null_mut();
            let enc_res = ffi::check(sys::heif_context_encode_image(
                ctx,
                image,
                encoder,
                options,
                &mut handle,
            ));
            if !options.is_null() {
                sys::heif_encoding_options_free(options);
            }
            if !handle.is_null() {
                sys::heif_image_handle_release(handle);
            }
            enc_res.map_err(|m| EncodeError::Heif(HeifError::Encode(m)))?;

            // Serialize the container to memory via a writer callback.
            let mut out: Vec<u8> = Vec::new();
            let mut writer = sys::heif_writer {
                writer_api_version: 1,
                write: Some(write_callback),
            };
            ffi::check(sys::heif_context_write(
                ctx,
                &mut writer,
                &mut out as *mut Vec<u8> as *mut c_void,
            ))
            .map_err(|m| EncodeError::Heif(HeifError::Encode(m)))?;

            Ok(out)
        }
    }

    /// Writes the converted, scaled samples into the interleaved plane row by row.
    #[allow(clippy::too_many_arguments)]
    fn fill_plane(
        &self,
        plane: *mut u8,
        stride: usize,
        buf: &[u8],
        layout: &Layout,
        width: u32,
        height: u32,
        out_depth: u32,
        out_sample_bytes: usize,
    ) {
        let src_bits = (layout.src_sample_bytes * 8) as u32;
        let in_row_bytes = width as usize * layout.src_channels * layout.src_sample_bytes;

        for y in 0..height as usize {
            let in_row = y * in_row_bytes;
            let out_row = y * stride;

            for x in 0..width as usize {
                let in_pixel = in_row + x * layout.src_channels * layout.src_sample_bytes;
                let out_pixel = out_row + x * layout.out_channels * out_sample_bytes;

                for c in 0..layout.out_channels {
                    // Map the output channel back to an input channel: grayscale replicates luma (channel 0) into R/G/B
                    // and keeps alpha as the last channel.
                    let src_channel = if layout.gray {
                        if layout.alpha && c == 3 { 1 } else { 0 }
                    } else {
                        c
                    };

                    let in_off = in_pixel + src_channel * layout.src_sample_bytes;
                    let value = scale(read_sample(buf, in_off, layout.src_sample_bytes), src_bits, out_depth);

                    let out_off = out_pixel + c * out_sample_bytes;
                    // SAFETY: `out_off + out_sample_bytes <= stride*height` by construction; the plane was allocated
                    // for `width`×`height` at `out_channels`.
                    unsafe {
                        if out_sample_bytes == 1 {
                            *plane.add(out_off) = value as u8;
                        } else {
                            let bytes = (value as u16).to_le_bytes();
                            *plane.add(out_off) = bytes[0];
                            *plane.add(out_off + 1) = bytes[1];
                        }
                    }
                }
            }
        }
    }
}

/// Sets a string parameter on the encoder, mapping libheif failures to [`EncodeError`].
///
/// # Safety
/// `encoder` must be a valid libheif encoder handle.
unsafe fn set_string_param(encoder: *mut sys::heif_encoder, name: &str, value: &str) -> Result<(), EncodeError> {
    let name = CString::new(name).expect("parameter name has no interior NUL");
    let value = CString::new(value).expect("parameter value has no interior NUL");
    // SAFETY: both C strings outlive the call; `encoder` is valid per contract.
    unsafe {
        ffi::check(sys::heif_encoder_set_parameter_string(
            encoder,
            name.as_ptr(),
            value.as_ptr(),
        ))
        .map_err(|m| EncodeError::Heif(HeifError::Encode(m)))
    }
}

/// libheif writer callback: appends the encoded bytes to the `Vec<u8>` behind `userdata`.
unsafe extern "C" fn write_callback(
    _ctx: *mut sys::heif_context,
    data: *const c_void,
    size: usize,
    userdata: *mut c_void,
) -> sys::heif_error {
    // SAFETY: `userdata` is the `&mut Vec<u8>` we passed to `heif_context_write`, and `data`/`size` describe a valid
    // buffer for the duration of the call.
    unsafe {
        let out = &mut *(userdata as *mut Vec<u8>);
        if !data.is_null() && size > 0 {
            out.extend_from_slice(std::slice::from_raw_parts(data as *const u8, size));
        }
        // A zeroed `heif_error` is `heif_error_Ok` with a null (auto-filled) message.
        std::mem::zeroed::<sys::heif_error>()
    }
}

/// Internal encode failure, distinguishing unsupported inputs (which become `ImageError::Unsupported`) from
/// libheif/runtime failures.
enum EncodeError {
    Unsupported(ExtendedColorType),
    Heif(HeifError),
}

impl From<EncodeError> for ImageError {
    fn from(err: EncodeError) -> Self {
        match err {
            EncodeError::Unsupported(color_type) => ImageError::Unsupported(UnsupportedError::from_format_and_kind(
                ImageFormatHint::Name("HEIF".into()),
                UnsupportedErrorKind::Color(color_type),
            )),
            EncodeError::Heif(e) => ImageError::Encoding(EncodingError::new(ImageFormatHint::Name("HEIF".into()), e)),
        }
    }
}

impl<W: Write> ImageEncoder for HeifEncoder<W> {
    fn write_image(mut self, buf: &[u8], width: u32, height: u32, color_type: ExtendedColorType) -> ImageResult<()> {
        let encoded = self.config.encode(buf, width, height, color_type)?;
        self.writer.write_all(&encoded).map_err(ImageError::IoError)?;
        Ok(())
    }
}
