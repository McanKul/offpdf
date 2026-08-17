# heif-rs

A Rust library to encode and decode HEIF/HEIC images using x265 and libde265, via statically-linked libheif.

## ⬇️ Installation

This library can be installed using Cargo. To do that, run the following command in your project's root directory:

```bash
cargo add heif-rs
```

The crate links as `heif`, so you import it with `use heif;` regardless of the package name.

> [!NOTE]
> The first build downloads the prebuilt static binaries for your platform, so an internet connection is required (see [Troubleshooting](#-troubleshooting) for offline builds).

## 🤖 Usage

Here are some examples of how to encode and decode HEIC images using this library. These snippets don't have any error handling for the sake of simplicity, but you should always check for errors in production code.

#### Encoding

```rust
let img = image::open("/path/to/image.png").unwrap(); // an image to be encoded
let bytes = heif::encode(&img).unwrap(); // encode the image with default settings
std::fs::write("/path/to/image.heic", &bytes).unwrap(); // save the HEIC to a file
```

#### Encoding with custom settings

```rust
use heif::{Chroma, HeifEncoder, Preset};
use image::ImageEncoder;

let img = image::open("/path/to/image.png").unwrap();
let mut bytes = Vec::new();
img.write_with_encoder(
    HeifEncoder::new(&mut bytes)
        .with_quality(80)             // 0–100, higher = better quality
        .with_preset(Preset::Slow)    // x265 speed preset (slower = better compression)
        .with_chroma(Chroma::Yuv420), // 4:2:0 / 4:2:2 / 4:4:4
).unwrap();
```

#### Decoding

```rust
let bytes = std::fs::read("/path/to/image.heic").unwrap(); // read the HEIC file
let img = heif::decode(&bytes).unwrap(); // decode it into a DynamicImage
img.save("/path/to/image.png").unwrap(); // save it in another format
```

#### Probing (header only)

Read the image dimensions and bit depth without decoding the pixels — useful for validation or thumbnailing pipelines:

```rust
let bytes = std::fs::read("/path/to/image.heic").unwrap();
let info = heif::probe(&bytes).unwrap();
println!("{}x{} @ {:?}", info.width, info.height, info.bit_depth);
```

The public API also exposes [`encode_buffer`] (encode a typed `ImageBuffer` directly), [`HeifEncoder`] / [`HeifDecoder`] for `image`-trait integration, [`EncoderConfig`] / [`DecoderConfig`] for full control, and [`libheif_version`].

#### Runnable examples

The [`examples/`](examples) directory has standalone programs covering each part of the API, runnable out of the box against the bundled assets:

```bash
cargo run --example encode          # encode with defaults
cargo run --example decode          # decode a HEIC to PNG
cargo run --example custom_encoder  # HeifEncoder builder (quality/preset/chroma)
cargo run --example encode_buffer   # encode a typed ImageBuffer
cargo run --example probe           # read the header without decoding pixels
cargo run --example roundtrip       # encode then decode
cargo run --example high_bit_depth  # 10-bit encoding via EncoderConfig
cargo run --example parallel_encode # concurrent encoding
cargo run --example version         # print the linked libheif version
```

## 💣 Troubleshooting

### High bit depth (10/12-bit) encoding fails

libheif supports 8/10/12-bit HEIC for both encode and decode. **Decoding** high-bit-depth HEIC works out of the box (via libde265). **Encoding** 10/12-bit, however, depends on the bundled x265 binary, which is currently an **8-bit build** — requesting 10/12-bit output makes libheif return an encoder error. If you need high-bit-depth encoding, supply your own libheif/x265 binaries built with high-bit-depth support via the `HEIF_BINARIES_DIR` environment variable (see below).

### My build fails because it can't download the binaries

The first build fetches the prebuilt static libraries for your platform over the network. For offline or air-gapped builds, download the archive for your target from [binaries-heif](https://github.com/vegidio/binaries-heif/releases), extract it, and point the build at it with the `HEIF_BINARIES_DIR` environment variable:

```
$ HEIF_BINARIES_DIR=/path/to/extracted/libs cargo build
```

## 📝 License

**heif-rs** is released under the Apache 2.0 License. See [LICENSE](LICENSE) for details.

## 👨🏾‍💻 Author

Vinicius Egidio ([vinicius.io](http://vinicius.io))
