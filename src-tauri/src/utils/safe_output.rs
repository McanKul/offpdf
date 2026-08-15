//! Overwrite-safe output helpers for Edit PDF.
//!
//! Canonical-path comparison misses hard links (same inode, different names).
//! Compare file identity, write qpdf to a sibling temp file, then rename onto
//! the destination so the original inode is never truncated.

use crate::error::AppError;
use std::path::{Path, PathBuf};

/// True when `a` and `b` refer to the same file (inode / file index).
/// Missing path → false. If both exist but identity cannot be read, fail closed
/// (treat as the same file) so a hard-linked dest is never written through.
pub fn same_file_identity(a: &Path, b: &Path) -> bool {
    if !a.exists() || !b.exists() {
        return false;
    }
    match (file_id(a), file_id(b)) {
        (Some(x), Some(y)) => x == y,
        _ => true,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    vol: u64,
    idx: u64,
}

#[cfg(unix)]
fn file_id(path: &Path) -> Option<FileId> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some(FileId {
        vol: meta.dev(),
        idx: meta.ino(),
    })
}

/// Stable Windows identity via `GetFileInformationByHandle` (not the nightly
/// `MetadataExt::file_index` / `volume_serial_number` APIs).
#[cfg(windows)]
fn file_id(path: &Path) -> Option<FileId> {
    use std::fs::File;
    use std::os::windows::io::AsRawHandle;

    let file = File::open(path).ok()?;
    let mut info = ByHandleFileInformation::zeroed();
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
    if ok == 0 {
        return None;
    }
    let idx = (u64::from(info.n_file_index_high) << 32) | u64::from(info.n_file_index_low);
    Some(FileId {
        vol: u64::from(info.dw_volume_serial_number),
        idx,
    })
}

#[cfg(not(any(unix, windows)))]
fn file_id(path: &Path) -> Option<FileId> {
    let canon = path.canonicalize().ok()?;
    // Fold the path into the two integers so we still compare without nightly APIs.
    let s = canon.to_string_lossy();
    let mut vol = 0u64;
    let mut idx = 0u64;
    for (i, b) in s.as_bytes().iter().enumerate() {
        if i % 2 == 0 {
            vol = vol.wrapping_mul(16777619) ^ u64::from(*b);
        } else {
            idx = idx.wrapping_mul(16777619) ^ u64::from(*b);
        }
    }
    Some(FileId { vol, idx })
}

#[cfg(windows)]
#[repr(C)]
struct FileTime {
    dw_low_date_time: u32,
    dw_high_date_time: u32,
}

#[cfg(windows)]
#[repr(C)]
struct ByHandleFileInformation {
    dw_file_attributes: u32,
    ft_creation_time: FileTime,
    ft_last_access_time: FileTime,
    ft_last_write_time: FileTime,
    dw_volume_serial_number: u32,
    n_file_size_high: u32,
    n_file_size_low: u32,
    n_number_of_links: u32,
    n_file_index_high: u32,
    n_file_index_low: u32,
}

#[cfg(windows)]
impl ByHandleFileInformation {
    fn zeroed() -> Self {
        Self {
            dw_file_attributes: 0,
            ft_creation_time: FileTime {
                dw_low_date_time: 0,
                dw_high_date_time: 0,
            },
            ft_last_access_time: FileTime {
                dw_low_date_time: 0,
                dw_high_date_time: 0,
            },
            ft_last_write_time: FileTime {
                dw_low_date_time: 0,
                dw_high_date_time: 0,
            },
            dw_volume_serial_number: 0,
            n_file_size_high: 0,
            n_file_size_low: 0,
            n_number_of_links: 0,
            n_file_index_high: 0,
            n_file_index_low: 0,
        }
    }
}

#[cfg(windows)]
extern "system" {
    fn GetFileInformationByHandle(
        handle: *mut std::ffi::c_void,
        info: *mut ByHandleFileInformation,
    ) -> i32;
}

/// Sibling temp next to `dest`, e.g. `.offpdf-{unique}.pdf.tmp`.
pub fn sibling_temp_path(dest: &Path, unique: &str) -> Result<PathBuf, AppError> {
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::io("Could not create the output folder.", e))?;
    }
    let safe: String = unique
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    Ok(parent.join(format!(".offpdf-{safe}.pdf.tmp")))
}

/// Replace `dest` with `from` via a directory-entry swap. Never copy onto an
/// existing dest (that would truncate a hard-linked original inode). Callers
/// must identity-check first.
///
/// Unix: `rename` replaces atomically. Windows: `MoveFileExW` with
/// `MOVEFILE_REPLACE_EXISTING` so dest is not unlinked before the new file is
/// in place.
pub fn replace_file(from: &Path, dest: &Path) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        return replace_file_windows(from, dest);
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, dest).map_err(|e| AppError::io("Could not write the output file.", e))
    }
}

#[cfg(windows)]
fn replace_file_windows(from: &Path, dest: &Path) -> Result<(), AppError> {
    use std::os::windows::ffi::OsStrExt;
    let mut src: Vec<u16> = from.as_os_str().encode_wide().collect();
    src.push(0);
    let mut dst: Vec<u16> = dest.as_os_str().encode_wide().collect();
    dst.push(0);
    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    let ok = unsafe { MoveFileExW(src.as_ptr(), dst.as_ptr(), MOVEFILE_REPLACE_EXISTING) };
    if ok == 0 {
        return Err(AppError::io(
            "Could not write the output file.",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
extern "system" {
    fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "offpdf-safe-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_dest_is_not_same_file() {
        let dir = scratch("missing");
        let a = dir.join("a.pdf");
        std::fs::write(&a, b"aaa").unwrap();
        assert!(!same_file_identity(&a, &dir.join("nope.pdf")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn same_path_is_same_file() {
        let dir = scratch("samepath");
        let a = dir.join("a.pdf");
        std::fs::write(&a, b"aaa").unwrap();
        assert!(same_file_identity(&a, &a));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hard_link_is_same_file() {
        let dir = scratch("hardlink");
        let a = dir.join("orig.pdf");
        let b = dir.join("alias.pdf");
        std::fs::write(&a, b"original-bytes").unwrap();
        if std::fs::hard_link(&a, &b).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert!(same_file_identity(&a, &b), "hard link must match inode");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_files_are_not_same() {
        let dir = scratch("distinct");
        let a = dir.join("a.pdf");
        let b = dir.join("b.pdf");
        std::fs::write(&a, b"aaa").unwrap();
        std::fs::write(&b, b"bbb").unwrap();
        assert!(!same_file_identity(&a, &b));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_file_leaves_source_inode_untouched() {
        let dir = scratch("replace");
        let orig = dir.join("orig.pdf");
        let dest = dir.join("out.pdf");
        let tmp = dir.join(".offpdf-x.pdf.tmp");
        std::fs::write(&orig, b"ORIGINAL").unwrap();
        std::fs::write(&tmp, b"NEWFILE").unwrap();
        replace_file(&tmp, &dest).unwrap();
        assert_eq!(std::fs::read(&orig).unwrap(), b"ORIGINAL");
        assert_eq!(std::fs::read(&dest).unwrap(), b"NEWFILE");
        assert!(!tmp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_file_overwrites_unrelated_dest_via_rename_not_copy() {
        let dir = scratch("replace-exist");
        let dest = dir.join("out.pdf");
        let tmp = dir.join(".offpdf-x.pdf.tmp");
        std::fs::write(&dest, b"OLD").unwrap();
        std::fs::write(&tmp, b"NEW").unwrap();
        replace_file(&tmp, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"NEW");
        assert!(!tmp.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sibling_temp_lives_next_to_dest() {
        let dir = scratch("sib");
        let dest = dir.join("edited.pdf");
        let tmp = sibling_temp_path(&dest, "job-1").unwrap();
        assert_eq!(tmp.parent(), dest.parent());
        let name = tmp.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with(".offpdf-"));
        assert!(name.ends_with(".pdf.tmp"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_over_hardlinked_dest_does_not_clobber_original_inode() {
        let dir = scratch("hl-replace");
        let orig = dir.join("orig.pdf");
        let dest = dir.join("hard-out.pdf");
        let tmp = dir.join(".offpdf-y.pdf.tmp");
        std::fs::write(&orig, b"KEEP-ME").unwrap();
        if std::fs::hard_link(&orig, &dest).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        std::fs::write(&tmp, b"NEW").unwrap();
        replace_file(&tmp, &dest).unwrap();
        assert_eq!(std::fs::read(&orig).unwrap(), b"KEEP-ME");
        assert_eq!(std::fs::read(&dest).unwrap(), b"NEW");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
