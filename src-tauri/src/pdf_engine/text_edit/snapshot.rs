//! One bounded read per operation, hashed and parsed from the same bytes (SPEC §B.4): raw
//! preflight (`snapshot/preflight.rs`: xref chain; `snapshot/objects.rs`: nesting and `/Length`
//! chains), guarded lopdf load (object streams decoded under a budget, their members
//! nesting-checked), policy refusals, and the lopdf/qpdf page-map agreement check.

use crate::error::AppError;
use crate::pdf_engine::text_edit::decode::{self, DecodeBudget};
use crate::pdf_engine::text_edit::engines::QpdfPage;
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::reasons::{self, EditProblem, EditProblemCode, ProblemCtx};
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

pub(crate) mod headers;
mod objects;
pub(crate) mod preflight;

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0100_0000_01b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub len: u64,
    pub fnv: u64,
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:016x}-{:x}", self.fnv, self.len)
    }
}

impl std::str::FromStr for Fingerprint {
    type Err = ();
    /// Strict inverse of `Display`: 16 lowercase hex digits, `-`, the length in lowercase hex
    /// without leading zeros.
    fn from_str(s: &str) -> Result<Self, ()> {
        let (fnv, len) = s.split_once('-').ok_or(())?;
        let lower_hex =
            |t: &str| !t.is_empty() && t.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
        if fnv.len() != 16 || !lower_hex(fnv) || !lower_hex(len) || len.len() > 16 {
            return Err(());
        }
        if len.len() > 1 && len.starts_with('0') {
            return Err(());
        }
        Ok(Fingerprint {
            fnv: u64::from_str_radix(fnv, 16).map_err(|_| ())?,
            len: u64::from_str_radix(len, 16).map_err(|_| ())?,
        })
    }
}

pub struct SourceSnapshot {
    pub path: PathBuf,
    pub bytes: Arc<Vec<u8>>,      // the only read of the file
    pub fingerprint: Fingerprint, // FNV-1a-64 of `bytes` + len
    pub head_tail: (u64, u64),    // FNV of the first / last HEAD_TAIL_HASH_BYTES
    pub modified: Option<SystemTime>,
    pub doc: Document,        // parsed from `bytes`, never from `path`
    pub pages: Vec<ObjectId>, // index = 0-based page index (doc.get_pages() order)
}

impl SourceSnapshot {
    /// The file's length (also once `release_bytes` dropped the bytes).
    pub fn file_len(&self) -> usize {
        usize::try_from(self.fingerprint.len).unwrap_or(usize::MAX)
    }

    /// Drops the raw bytes once only the parsed document is read (a Save's source after its
    /// copy is written, Phase A/B's verification reads after the page map): the document holds
    /// every stream again, so keeping both doubled what a large file costs (review-final
    /// MEDIUM-1). Nothing past these points reads `bytes`.
    pub fn release_bytes(&mut self) {
        self.bytes = Arc::new(Vec::new());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Policy {
    Source,
    Verification,
}

pub(crate) fn fnv1a_extend(mut hash: u64, bytes: &[u8]) -> u64 {
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub(crate) fn fnv1a_u64(bytes: &[u8]) -> u64 {
    fnv1a_extend(FNV_OFFSET, bytes)
}

fn head_tail_hashes(bytes: &[u8]) -> (u64, u64) {
    let n = limits::HEAD_TAIL_HASH_BYTES;
    let head = bytes.get(..n.min(bytes.len())).unwrap_or_default();
    let tail = bytes
        .get(bytes.len().saturating_sub(n)..)
        .unwrap_or_default();
    (fnv1a_u64(head), fnv1a_u64(tail))
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Steps 1–2 of `read_snapshot`: regular file, size check, one capped read.
fn read_capped(path: &Path, cap: u64) -> Result<(Vec<u8>, Option<SystemTime>), AppError> {
    if !path.is_file() {
        return Err(AppError::invalid_pdf(&path_text(path)));
    }
    let meta = std::fs::metadata(path)
        .map_err(|e| AppError::invalid_pdf(&path_text(path)).with_details(e.to_string()))?;
    if meta.len() > cap {
        return Err(reasons::file_too_large());
    }
    let file =
        std::fs::File::open(path).map_err(|e| AppError::io("OffPDF could not open the PDF.", e))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(usize::try_from(meta.len().min(cap)).unwrap_or(0))
        .map_err(|_| reasons::file_too_large())?;
    file.take(cap.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|e| AppError::io("OffPDF could not read the PDF.", e))?;
    if bytes.len() as u64 > cap {
        return Err(reasons::file_too_large());
    }
    Ok((bytes, meta.modified().ok()))
}

/// Opens a source PDF for text editing: one capped read, preflight, guarded parse, policy.
pub fn read_snapshot(path: &Path) -> Result<SourceSnapshot, AppError> {
    let (bytes, modified) = read_capped(path, limits::file_cap())?;
    snapshot_from_bytes(path, bytes, modified)
}

/// `read_snapshot` from bytes already in memory (the caller did the one read; `read_snapshot`
/// itself goes through here, so tests that start from bytes run the production path).
pub fn snapshot_from_bytes(
    path: &Path,
    bytes: Vec<u8>,
    modified: Option<SystemTime>,
) -> Result<SourceSnapshot, AppError> {
    if bytes.len() as u64 > limits::file_cap() {
        return Err(reasons::file_too_large());
    }
    build(path, bytes, modified, Policy::Source)
}

/// Steps 1–6 and 8 of `read_snapshot` with `cap` instead of `file_cap()`, and **without** step 7's policy
/// refusals (encrypted, signed, XFA). Used only for files the pipeline wrote (edited copies, preview files,
/// the staged output). Every failure is `EDIT_VERIFY_FAILED` (an encrypted file cannot be read either).
pub fn read_verification_snapshot(path: &Path, cap: u64) -> Result<SourceSnapshot, AppError> {
    read_capped(path, cap)
        .and_then(|(bytes, modified)| build(path, bytes, modified, Policy::Verification))
        .map_err(verification_error)
}

fn verification_error(e: AppError) -> AppError {
    let lead = match e.code.as_str() {
        "FILE_TOO_LARGE" | "FILE_TOO_COMPLEX" => "too large to verify",
        _ => "could not read the checked file",
    };
    let detail = format!(
        "{lead}: {}: {}",
        e.code,
        e.details.clone().unwrap_or_else(|| e.message.clone())
    );
    let p = EditProblem::new(EditProblemCode::EditVerifyFailed, Some(detail));
    let ctx = ProblemCtx {
        page_number: None,
        file_name: None,
        face: None,
        reason: None,
    };
    EditProblemCode::EditVerifyFailed.to_app_error(&p, &ctx)
}

fn build(
    path: &Path,
    bytes: Vec<u8>,
    modified: Option<SystemTime>,
    policy: Policy,
) -> Result<SourceSnapshot, AppError> {
    let fingerprint = Fingerprint {
        len: bytes.len() as u64,
        fnv: fnv1a_u64(&bytes),
    };
    let head_tail = head_tail_hashes(&bytes);
    preflight::preflight(&bytes)?;
    let doc = guarded_load(path, &bytes)?;
    if document_is_encrypted(&doc) {
        return Err(reasons::encrypted());
    }
    if policy == Policy::Source {
        if document_is_signed(&doc) {
            return Err(reasons::signed());
        }
        if document_has_xfa(&doc) {
            return Err(reasons::unsupported_xfa());
        }
    }
    if doc.catalog().is_err() {
        return Err(reasons::malformed_content(
            "The PDF catalog is missing or unreadable.",
        ));
    }
    let pages = doc.get_pages().into_values().collect();
    Ok(SourceSnapshot {
        path: path.to_path_buf(),
        bytes: Arc::new(bytes),
        fingerprint,
        head_tail,
        modified,
        doc,
        pages,
    })
}

/// (len, mtime) unchanged on disk **and** the first/last HEAD_TAIL_HASH_BYTES hash to `head_tail`
/// (catches same-size rewrites on file systems with coarse mtime: FAT/exFAT, network shares).
pub fn stat_matches(snap: &SourceSnapshot) -> bool {
    let Ok(meta) = std::fs::metadata(&snap.path) else {
        return false;
    };
    if meta.len() != snap.fingerprint.len || meta.modified().ok() != snap.modified {
        return false;
    }
    let n = limits::HEAD_TAIL_HASH_BYTES as u64;
    let read_at = |offset: u64, len: u64| -> Option<Vec<u8>> {
        let mut f = std::fs::File::open(&snap.path).ok()?;
        f.seek(SeekFrom::Start(offset)).ok()?;
        let mut buf = Vec::new();
        f.take(len).read_to_end(&mut buf).ok()?;
        (buf.len() as u64 == len).then_some(buf)
    };
    let len = meta.len();
    let head = read_at(0, len.min(n));
    let tail = read_at(len.saturating_sub(n), len.min(n));
    match (head, tail) {
        (Some(h), Some(t)) => (fnv1a_u64(&h), fnv1a_u64(&t)) == snap.head_tail,
        _ => false,
    }
}

/// Page-map agreement (D14): `qpdf` must list the same page count, the same page object ids in order, and for
/// each page the same `/Contents` stream ids as lopdf. Mismatch → PDF_NEEDS_REPAIR (details: first difference).
pub fn check_page_map(snap: &SourceSnapshot, qpdf_pages: &[QpdfPage]) -> Result<(), AppError> {
    let repair = |what: String| {
        Err(reasons::pdf_needs_repair(&[format!(
            "page map disagreement: {what}"
        )]))
    };
    if snap.pages.len() != qpdf_pages.len() {
        return repair(format!(
            "lopdf reads {} pages, qpdf {}",
            snap.pages.len(),
            qpdf_pages.len()
        ));
    }
    for (i, (ours, theirs)) in snap.pages.iter().zip(qpdf_pages).enumerate() {
        let n = i + 1;
        if *ours != theirs.object {
            return repair(format!(
                "page {n}: lopdf object {} {} R, qpdf {} {} R",
                ours.0, ours.1, theirs.object.0, theirs.object.1
            ));
        }
        match page_contents_ids(&snap.doc, *ours) {
            Some(ids) if ids == theirs.contents => {}
            Some(ids) => {
                return repair(format!(
                    "page {n}: /Contents {ids:?} (lopdf) vs {:?} (qpdf)",
                    theirs.contents
                ))
            }
            None => return repair(format!("page {n}: /Contents unresolvable for lopdf")),
        }
    }
    Ok(())
}

/// The content stream ids of a page as qpdf lists them; `None` when lopdf cannot resolve them.
fn page_contents_ids(doc: &Document, page_id: ObjectId) -> Option<Vec<ObjectId>> {
    let page = doc.get_dictionary(page_id).ok()?;
    let refs_of = |items: &[Object]| -> Option<Vec<ObjectId>> {
        items
            .iter()
            .map(|o| {
                o.as_reference()
                    .ok()
                    .filter(|id| matches!(doc.objects.get(id), Some(Object::Stream(_))))
            })
            .collect()
    };
    match page.get(b"Contents").ok() {
        None | Some(Object::Null) => Some(Vec::new()),
        Some(Object::Reference(id)) => match doc.objects.get(id)? {
            Object::Stream(_) => Some(vec![*id]),
            Object::Array(items) => refs_of(items),
            _ => None,
        },
        Some(Object::Array(items)) => refs_of(items),
        Some(_) => None,
    }
}

fn document_is_encrypted(doc: &Document) -> bool {
    doc.is_encrypted() || doc.trailer.get(b"Encrypt").is_ok_and(|o| !o.is_null())
}

/// Catalog `/Perms`, or any `/Type /Sig` or `/FT /Sig` dictionary **with** `/ByteRange`
/// (same rule as the #33 classifier; empty signature widgets are fine).
pub(crate) fn document_is_signed(doc: &Document) -> bool {
    if doc.catalog().is_ok_and(|cat| cat.has(b"Perms")) {
        return true;
    }
    doc.objects.values().any(|obj| {
        let dict = match obj {
            Object::Dictionary(d) => d,
            Object::Stream(s) => &s.dict,
            _ => return false,
        };
        (name_is(dict, b"Type", b"Sig") || name_is(dict, b"FT", b"Sig")) && dict.has(b"ByteRange")
    })
}

fn name_is(dict: &Dictionary, key: &[u8], expect: &[u8]) -> bool {
    dict.get(key).ok().and_then(|o| o.as_name().ok()) == Some(expect)
}

/// `/AcroForm /XFA` present, or `/NeedsRendering true` (same rule as Fill forms' `detect_xfa`).
pub(crate) fn document_has_xfa(doc: &Document) -> bool {
    let Ok(cat) = doc.catalog() else { return false };
    let needs_rendering = |d: &Dictionary| match d.get(b"NeedsRendering") {
        Ok(Object::Boolean(true)) => true,
        Ok(Object::Integer(i)) => *i != 0,
        _ => false,
    };
    let acro = match cat.get(b"AcroForm") {
        Ok(Object::Reference(id)) => doc.get_dictionary(*id).ok(),
        Ok(Object::Dictionary(d)) => Some(d),
        _ => None,
    };
    needs_rendering(cat)
        || acro.is_some_and(|a| a.get(b"XFA").is_ok_and(|x| !x.is_null()) || needs_rendering(a))
}

// ---- Guarded lopdf load ------------------------------------------------------------------

static LOAD_LOCK: Mutex<()> = Mutex::new(());
static OBJSTM_BUDGET: AtomicUsize = AtomicUsize::new(0);
static OBJSTM_SCAN_BUDGET: AtomicUsize = AtomicUsize::new(0);
static OBJSTM_REJECTED: AtomicBool = AtomicBool::new(false);
const REJECTED_OBJSTM: &[u8] = b"OffPdfRejectedObjStm";

fn guarded_load(path: &Path, bytes: &[u8]) -> Result<Document, AppError> {
    let _lock = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    OBJSTM_BUDGET.store(limits::OBJSTM_TOTAL_DECODED, Ordering::SeqCst);
    OBJSTM_SCAN_BUDGET.store(limits::objstm_scan_budget(), Ordering::SeqCst);
    OBJSTM_REJECTED.store(false, Ordering::SeqCst);
    let doc = lopdf::Reader {
        buffer: bytes,
        document: Document::new(),
    }
    .read(Some(objstm_guard))
    .map_err(|e| AppError::invalid_pdf(&path_text(path)).with_details(format!("lopdf: {e}")))?;
    let rejected = OBJSTM_REJECTED.load(Ordering::SeqCst)
        || doc
            .objects
            .values()
            .any(|o| matches!(o, Object::Stream(s) if s.dict.type_is(REJECTED_OBJSTM)));
    if rejected {
        return Err(reasons::file_too_complex(
            "an object stream is too large, too deeply nested or cannot be decoded",
        ));
    }
    if doc.objects.len() > limits::MAX_OBJECTS {
        return Err(reasons::file_too_complex("too many objects"));
    }
    Ok(doc)
}

/// lopdf filter: decodes object streams under the per-stream and total budgets (so lopdf's own
/// unbounded `decompress` becomes a no-op) and nesting-checks the members lopdf will parse from
/// them under one scan budget per load, rejects the rest, and never clones a stream.
fn objstm_guard(id: (u32, u16), obj: &mut Object) -> Option<((u32, u16), Object)> {
    let Object::Stream(stream) = obj else {
        // Inside object streams lopdf uses the returned object; at top level it is ignored.
        return Some((id, obj.clone()));
    };
    if stream.dict.type_is(b"ObjStm") {
        let remaining = OBJSTM_BUDGET.load(Ordering::SeqCst);
        let cap = limits::OBJSTM_MAX_DECODED.min(remaining);
        let declared = stream.dict.get(b"N").and_then(Object::as_i64).unwrap_or(0);
        let decoded = decode::decode_stream(stream, cap, &mut DecodeBudget::new(usize::MAX));
        let accepted = match decoded {
            Ok(data)
                if !(data.is_empty() && declared > 0)
                    && objects::objstm_members_ok(&stream.dict, &data, &OBJSTM_SCAN_BUDGET) =>
            {
                let debited = OBJSTM_BUDGET
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |r| {
                        r.checked_sub(data.len())
                    })
                    .is_ok();
                if debited {
                    stream.dict.remove(b"Filter");
                    stream.dict.remove(b"DecodeParms");
                    stream.set_content(data);
                }
                debited
            }
            _ => false,
        };
        if !accepted {
            stream
                .dict
                .set("Type", Object::Name(REJECTED_OBJSTM.to_vec()));
            stream.content.clear();
            OBJSTM_REJECTED.store(true, Ordering::SeqCst);
        }
    }
    // Streams are always top level, where lopdf ignores the returned value: never clone them.
    Some((id, Object::Null))
}
