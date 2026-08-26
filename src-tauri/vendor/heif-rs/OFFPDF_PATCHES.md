# OffPDF patches

This directory vendors `heif-rs` 26.7.0 (Apache-2.0) because OffPDF decodes
untrusted HEIC/HEIF files through its native libheif FFI boundary.

The local decoder patch:

- rejects null handles and decoded-image pointers returned across FFI;
- uses libheif's `size_t`-based `heif_image_get_plane_readonly2` API instead
  of the deprecated `int`-stride variant;
- validates row stride, destination length, and pointer-offset arithmetic
  before copying decoded pixels;
- checks decoded interleaved plane size against the handle before copy; and
- releases a partially returned image on decode errors.

Keep the original `LICENSE` file and rebase these changes when upgrading the
vendored crate. Remove the vendored copy once an equivalent upstream release is
available and has passed OffPDF's real-device Windows regression test.
