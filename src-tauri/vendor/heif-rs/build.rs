//! Build script for `heif-rs`.
//!
//! At build time this script obtains the prebuilt **static** libheif binaries (and the codec/support libraries it
//! depends on) for the current target, links them statically into the crate, and generates the raw FFI bindings from
//! the bundled `heif.h` header.
//!
//! Binaries come from: https://github.com/vegidio/binaries-heif/releases
//!
//! The binaries are normally downloaded from the pinned release and cached under the build's `OUT_DIR`. To build
//! offline (or against a custom build of libheif), set the `HEIF_BINARIES_DIR` environment variable to a directory that
//! contains `include/` and `lib/` subdirectories laid out like the release archives.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Version of the `binaries-heif` release to download.
const VERSION: &str = "26.7.0";

/// Static archives ship these libraries. Listed dependents-before-dependencies so that GNU ld's single-pass
/// resolution finds every symbol: libheif uses x265 to encode HEVC and libde265 to decode it. These are the
/// macOS/Linux/mingw (`.a`) basenames; the MSVC `.lib` archive names them differently — see `emit_link_directives`.
const STATIC_LIBS: &[&str] = &["heif", "x265", "de265"];

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=HEIF_BINARIES_DIR");

    let binaries_dir = locate_binaries();
    let lib_dir = binaries_dir.join("lib");
    let include_dir = binaries_dir.join("include");

    emit_link_directives(&lib_dir);
    generate_bindings(&include_dir);
}

/// Returns the directory containing the `include/` and `lib/` subdirectories, either from the `HEIF_BINARIES_DIR`
/// override or by downloading + extracting the pinned release.
fn locate_binaries() -> PathBuf {
    if let Ok(dir) = env::var("HEIF_BINARIES_DIR") {
        let dir = PathBuf::from(dir);
        assert!(
            dir.join("include").is_dir() && dir.join("lib").is_dir(),
            "HEIF_BINARIES_DIR ({}) must contain `include/` and `lib/` subdirectories",
            dir.display()
        );
        return dir;
    }

    download_and_extract()
}

/// Downloads the static archive for the current target into `OUT_DIR` and extracts it.
/// Extraction is skipped if a previous build already populated the cache directory.
fn download_and_extract() -> PathBuf {
    let archive = archive_name();
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));
    let cache_dir = out_dir.join(format!("binaries-heif-{VERSION}"));

    // Idempotent: a fully extracted cache from a previous build is reused as-is.
    if cache_dir.join("lib").is_dir() && cache_dir.join("include").is_dir() {
        return cache_dir;
    }

    let url = format!("https://github.com/vegidio/binaries-heif/releases/download/{VERSION}/{archive}");
    eprintln!("heif-rs: downloading {url}");

    let bytes = download(&url);
    extract_zip(&bytes, &cache_dir);
    cache_dir
}

/// Maps the Cargo target triple components to the release archive file name,
/// e.g. `static_osx_arm64.zip`.
fn archive_name() -> String {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    let os = match target_os.as_str() {
        "linux" => "linux",
        "macos" => "osx",
        "windows" => "windows",
        other => panic!("heif-rs: unsupported target OS `{other}`"),
    };

    let arch = match target_arch.as_str() {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => panic!("heif-rs: unsupported target architecture `{other}`"),
    };

    format!("static_{os}_{arch}.zip")
}

/// Downloads the given URL into memory, following redirects.
fn download(url: &str) -> Vec<u8> {
    let mut reader = ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("heif-rs: failed to download {url}: {e}"))
        .into_body()
        .into_reader();

    let mut bytes = Vec::new();
    io::copy(&mut reader, &mut bytes)
        .unwrap_or_else(|e| panic!("heif-rs: failed to read response body from {url}: {e}"));
    bytes
}

/// Extracts a zip archive (held entirely in memory) into `dest`.
fn extract_zip(bytes: &[u8], dest: &Path) {
    let reader = io::Cursor::new(bytes);
    let mut zip = zip::ZipArchive::new(reader).expect("heif-rs: invalid zip archive");

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).expect("heif-rs: corrupt zip entry");
        let Some(rel_path) = entry.enclosed_name() else {
            continue; // skip unsafe / absolute paths
        };
        let out_path = dest.join(rel_path);

        if entry.is_dir() {
            fs::create_dir_all(&out_path).unwrap();
            continue;
        }

        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut out_file = fs::File::create(&out_path)
            .unwrap_or_else(|e| panic!("heif-rs: cannot create {}: {e}", out_path.display()));
        io::copy(&mut entry, &mut out_file).expect("heif-rs: failed to extract file");
    }
}

/// Tells Cargo/rustc where the static libraries live and which ones to link, including the C++ runtime and system
/// libraries the codecs depend on.
fn emit_link_directives(lib_dir: &Path) {
    println!("cargo:rustc-link-search=native={}", lib_dir.display());

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();

    // The MSVC Windows archive names its static libs `heif.lib`, `x265-static.lib` and
    // `libde265.lib`, so the link names differ from the `.a` archives used everywhere else.
    let libs: &[&str] = if target_os == "windows" {
        &["heif", "x265-static", "libde265"]
    } else {
        STATIC_LIBS
    };

    for lib in libs {
        println!("cargo:rustc-link-lib=static={lib}");
    }

    // libheif, x265, and libde265 are compiled C++, so the C++ runtime and a few system
    // libs must be linked.
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    match target_os.as_str() {
        "macos" => {
            println!("cargo:rustc-link-lib=dylib=c++");
            // On x86_64, libde265's runtime CPU-feature detection (`__builtin_cpu_supports`)
            // references `__cpu_model` / `__cpu_indicator_init`, which live in clang's
            // compiler-rt builtins rather than libSystem. rustc links with `-nodefaultlibs`, so
            // they must be pulled in explicitly. (arm64 has no such CPUID path.)
            if target_arch == "x86_64" {
                link_compiler_rt_builtins();
            }
        }
        "linux" => {
            println!("cargo:rustc-link-lib=dylib=stdc++");
            println!("cargo:rustc-link-lib=dylib=m");
            println!("cargo:rustc-link-lib=dylib=pthread");
            println!("cargo:rustc-link-lib=dylib=dl");
        }
        "windows" => {
            // The release ships MSVC `.lib` archives, which link the C/C++ runtime
            // automatically via the objects' default-lib directives. x265's thread pool, however,
            // reads the Windows registry (`RegOpenKeyExA` etc.), so its home — advapi32 — must be
            // linked explicitly.
            println!("cargo:rustc-link-lib=dylib=advapi32");
        }
        _ => {}
    }
}

/// Links clang's compiler-rt builtins so the x86 CPU-feature symbols (`__cpu_model` /
/// `__cpu_indicator_init`) that libde265 references resolve. Locates the archive via
/// `clang -print-runtime-dir`; silently does nothing if clang can't be queried, in which case the
/// linker simply reports the missing symbols as before.
fn link_compiler_rt_builtins() {
    let cc = env::var("CC").unwrap_or_else(|_| "clang".into());
    let Ok(output) = std::process::Command::new(cc).arg("-print-runtime-dir").output() else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if dir.is_empty() {
        return;
    }
    // `libclang_rt.osx.a` provides the builtins for every macOS arch.
    println!("cargo:rustc-link-search=native={dir}");
    println!("cargo:rustc-link-lib=static=clang_rt.osx");
}

/// Generates raw FFI bindings from `include/libheif/heif.h` into `OUT_DIR/bindings.rs`.
fn generate_bindings(include_dir: &Path) {
    let header = include_dir.join("libheif").join("heif.h");
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let bindings = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .clang_arg(format!("-I{}", include_dir.display()))
        // Keep the output focused on the libheif surface.
        .allowlist_function("heif_.*")
        .allowlist_type("heif_.*")
        .allowlist_var("heif_.*")
        .allowlist_var("LIBHEIF.*")
        .generate_comments(false)
        .layout_tests(false)
        .generate()
        .expect("heif-rs: failed to generate bindings from heif.h");

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("heif-rs: failed to write bindings.rs");
}
