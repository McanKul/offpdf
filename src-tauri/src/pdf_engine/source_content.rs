//! Read-only source-content classifier (#33).
//!
//! Walks page streams and Form XObjects on the original source path. Does not
//! write a dest, call `Document::replace_text`, or apply page `/Rotate`.
//!
//! Research prototype only; no Tauri command or UI invokes this module.
//! Decoded-size checks currently run after allocation, and font/geometry
//! support is incomplete. Complete #33's resource bounds and compatibility
//! evaluation before exposing this API to user files or enabling editing.

use crate::error::AppError;
use crate::pdf_engine::crop;
use lopdf::{content::Content, Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashMap;
use std::path::Path;

const FILE_CAP_BYTES: u64 = 400 * 1024 * 1024;
const MAX_FORM_DEPTH: usize = 8;
const MAX_STREAM_BYTES: usize = 32 * 1024 * 1024;
const MAX_DECODED_TOTAL: usize = 64 * 1024 * 1024;
const MAX_OPS: usize = 50_000;
const MAX_OCCURRENCES: usize = 5_000;
const MAX_GSTATE_STACK: usize = 64;

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Text,
    Image,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceCapability {
    Supported,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SourceOccurrence {
    pub page_index: u32,
    pub kind: SourceKind,
    pub rect: SourceRect,
    pub locator: String,
    pub capability: SourceCapability,
    pub reason: Option<String>,
}

pub fn classify_source_content(path: &Path) -> Result<Vec<SourceOccurrence>, AppError> {
    let (doc, fp) = open_source(path)?;
    classify_doc(&doc, fp)
}

pub fn resolve_source_locator(
    path: &Path,
    locator: &str,
) -> Result<SourceOccurrence, AppError> {
    if !path.is_file() {
        return Err(AppError::invalid_pdf(&path_str(path)));
    }
    let meta = std::fs::metadata(path).map_err(|_| AppError::invalid_pdf(&path_str(path)))?;
    if meta.len() > FILE_CAP_BYTES {
        return Err(file_too_large());
    }
    let bytes = std::fs::read(path).map_err(|_| AppError::invalid_pdf(&path_str(path)))?;
    let fp = fnv1a_u64(&bytes);
    let loc_fp = parse_locator_fp(locator).ok_or_else(stale)?;
    if loc_fp != fp {
        return Err(stale());
    }
    classify_source_content(path)?
        .into_iter()
        .find(|o| o.locator == locator)
        .ok_or_else(stale)
}

fn open_source(path: &Path) -> Result<(Document, u64), AppError> {
    if !path.is_file() {
        return Err(AppError::invalid_pdf(&path_str(path)));
    }
    let meta = std::fs::metadata(path).map_err(|_| AppError::invalid_pdf(&path_str(path)))?;
    if meta.len() > FILE_CAP_BYTES {
        return Err(file_too_large());
    }
    let bytes = std::fs::read(path).map_err(|_| AppError::invalid_pdf(&path_str(path)))?;
    let fp = fnv1a_u64(&bytes);
    let doc = Document::load(path).map_err(|e| {
        AppError::invalid_pdf(&path_str(path)).with_details(format!("lopdf: {e}"))
    })?;
    if doc.is_encrypted() {
        return Err(encrypted());
    }
    if document_is_signed(&doc) {
        return Err(signed());
    }
    if doc.catalog().is_err() {
        return Err(malformed_content("The PDF catalog is missing or unreadable."));
    }
    Ok((doc, fp))
}

fn classify_doc(doc: &Document, fp: u64) -> Result<Vec<SourceOccurrence>, AppError> {
    let mut walker = Walker {
        doc,
        fp,
        decoded_total: 0,
        op_count: 0,
        pending: Vec::new(),
        paint_counts: HashMap::new(),
    };
    let pages = doc.get_pages();
    let mut nums: Vec<u32> = pages.keys().copied().collect();
    nums.sort_unstable();
    for num in nums {
        let Some(&page_id) = pages.get(&num) else {
            continue;
        };
        walker.walk_page(page_id, num.saturating_sub(1))?;
    }
    Ok(walker.finish())
}

struct Walker<'a> {
    doc: &'a Document,
    fp: u64,
    decoded_total: usize,
    op_count: usize,
    pending: Vec<Pending>,
    paint_counts: HashMap<ObjectId, usize>,
}

#[derive(Clone, Copy, Default)]
struct Flags {
    inline_image: bool,
    nested_form: bool,
    type3: bool,
    vertical: bool,
    clipped: bool,
    pattern: bool,
    rotated: bool,
    skewed: bool,
    no_tounicode: bool,
    missing_font: bool,
    ambiguous: bool,
    masked: bool,
    shared: bool,
    geometry: bool,
}

struct Pending {
    page_index: u32,
    kind: SourceKind,
    rect: SourceRect,
    locator: String,
    flags: Flags,
    paint_id: Option<ObjectId>,
}

#[derive(Clone)]
struct GState {
    ctm: [f64; 6],
    clip_active: bool,
    clip_pending: bool,
    fill_pattern: bool,
    stroke_pattern: bool,
    masked: bool,
    font_name: Option<Vec<u8>>,
    font_size: f64,
    leading: f64,
    hscale: f64,
    tc: f64,
    tw: f64,
}

impl Default for GState {
    fn default() -> Self {
        Self {
            ctm: IDENTITY,
            clip_active: false,
            clip_pending: false,
            fill_pattern: false,
            stroke_pattern: false,
            masked: false,
            font_name: None,
            font_size: 0.0,
            leading: 0.0,
            hscale: 100.0,
            tc: 0.0,
            tw: 0.0,
        }
    }
}

impl GState {
    fn pattern(&self) -> bool {
        self.fill_pattern || self.stroke_pattern
    }

    fn clipped(&self) -> bool {
        self.clip_active || self.clip_pending
    }
}

struct TextState {
    tm: [f64; 6],
    tlm: [f64; 6],
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            tm: IDENTITY,
            tlm: IDENTITY,
        }
    }
}

struct FormInfo {
    matrix: [f64; 6],
    bytes: Vec<u8>,
    has_resources: bool,
}

struct TextInspect {
    missing_font: bool,
    type3: bool,
    vertical: bool,
    no_tounicode: bool,
    ambiguous: bool,
    width: f64,
    font_id: ObjectId,
}

impl Walker<'_> {
    fn walk_page(&mut self, page_id: ObjectId, page_index: u32) -> Result<(), AppError> {
        let geom_unsafe = page_geom_unsafe(self.doc, page_id);
        let owners = page_resource_owners(self.doc, page_id);
        let ids = self.doc.get_page_contents(page_id);
        if ids.is_empty() {
            return Ok(());
        }
        let contents_id = ids[0];
        let mut bytes = Vec::new();
        for id in &ids {
            let stream = self
                .doc
                .get_object(*id)
                .ok()
                .and_then(|o| o.as_stream().ok())
                .ok_or_else(|| malformed_content("A page content stream is unreadable."))?;
            let chunk = decompress_stream(stream)?;
            self.add_decoded(chunk.len())?;
            if !bytes.is_empty() {
                bytes.push(b'\n');
            }
            bytes.extend_from_slice(&chunk);
        }
        let mut visiting = Vec::new();
        self.walk_stream(
            page_index,
            contents_id,
            &bytes,
            &owners,
            GState::default(),
            0,
            &mut visiting,
            geom_unsafe,
        )
    }

    fn walk_stream(
        &mut self,
        page_index: u32,
        contents_id: ObjectId,
        bytes: &[u8],
        resource_owners: &[ObjectId],
        mut gs: GState,
        form_depth: usize,
        visiting: &mut Vec<ObjectId>,
        geom_unsafe: bool,
    ) -> Result<(), AppError> {
        let inline_total = count_inline_images(bytes)?;
        // BI…EI: do not trust a prefix Content::decode Ok; strip payloads first.
        let stripped = if inline_total > 0 {
            Some(strip_inline_image_payloads(bytes)?)
        } else {
            None
        };
        let decode_src = stripped.as_deref().unwrap_or(bytes);
        let ops = match Content::decode(decode_src) {
            Ok(c) => c.operations,
            Err(_) => {
                return Err(malformed_content(
                    "A page content stream could not be decoded.",
                ));
            }
        };
        self.bump_ops(ops.len())?;

        let mut ts = TextState::default();
        let mut stack: Vec<GState> = Vec::new();
        let mut inline_emitted = 0usize;
        let nested = form_depth > 0;

        for (op_index, op) in ops.iter().enumerate() {
            match op.operator.as_str() {
                "q" => {
                    if stack.len() < MAX_GSTATE_STACK {
                        stack.push(gs.clone());
                    }
                }
                "Q" => {
                    if let Some(prev) = stack.pop() {
                        gs = prev;
                    }
                }
                "cm" => {
                    if let Some(m) = six_nums(&op.operands) {
                        gs.ctm = mul(gs.ctm, m);
                    }
                }
                "W" | "W*" => {
                    gs.clip_pending = true;
                }
                "n" => {
                    if gs.clip_pending {
                        gs.clip_active = true;
                        gs.clip_pending = false;
                    }
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" => {
                    if gs.clip_pending {
                        gs.clip_active = true;
                        gs.clip_pending = false;
                    }
                }
                "cs" => {
                    gs.fill_pattern =
                        operand_selects_pattern_space(self.doc, resource_owners, &op.operands);
                }
                "CS" => {
                    gs.stroke_pattern =
                        operand_selects_pattern_space(self.doc, resource_owners, &op.operands);
                }
                "scn" | "sc" => {
                    if gs.fill_pattern || is_pattern_name(&op.operands) {
                        gs.fill_pattern = true;
                    }
                }
                "SCN" | "SC" => {
                    if gs.stroke_pattern || is_pattern_name(&op.operands) {
                        gs.stroke_pattern = true;
                    }
                }
                "gs" => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        apply_extgstate_smask(self.doc, resource_owners, name, &mut gs);
                    }
                }
                "rg" | "g" | "k" => {
                    gs.fill_pattern = false;
                }
                "RG" | "G" | "K" => {
                    gs.stroke_pattern = false;
                }
                "BT" => {
                    ts.tm = IDENTITY;
                    ts.tlm = IDENTITY;
                }
                "ET" => {}
                "Tf" => {
                    if let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        gs.font_name = Some(name.to_vec());
                    }
                    if let Some(size) = op.operands.get(1).and_then(obj_f64) {
                        gs.font_size = size;
                    }
                }
                "Tc" => {
                    if let Some(c) = op.operands.first().and_then(obj_f64) {
                        gs.tc = c;
                    }
                }
                "Tw" => {
                    if let Some(w) = op.operands.first().and_then(obj_f64) {
                        gs.tw = w;
                    }
                }
                "Tz" => {
                    if let Some(z) = op.operands.first().and_then(obj_f64) {
                        gs.hscale = z;
                    }
                }
                "TL" => {
                    if let Some(l) = op.operands.first().and_then(obj_f64) {
                        gs.leading = l;
                    }
                }
                "Td" => {
                    if let Some((tx, ty)) = two_nums(&op.operands) {
                        apply_td(&mut ts, tx, ty);
                    }
                }
                "TD" => {
                    if let Some((tx, ty)) = two_nums(&op.operands) {
                        gs.leading = -ty;
                        apply_td(&mut ts, tx, ty);
                    }
                }
                "Tm" => {
                    if let Some(m) = six_nums(&op.operands) {
                        ts.tm = m;
                        ts.tlm = m;
                    }
                }
                "T*" => {
                    let ty = -gs.leading;
                    apply_td(&mut ts, 0.0, ty);
                }
                "Tj" | "'" | "\"" | "TJ" => {
                    let mut show_ops = op.operands.as_slice();
                    if op.operator == "\"" {
                        if let Some(aw) = op.operands.first().and_then(obj_f64) {
                            gs.tw = aw;
                        }
                        if let Some(ac) = op.operands.get(1).and_then(obj_f64) {
                            gs.tc = ac;
                        }
                        if op.operands.len() >= 2
                            && obj_f64(&op.operands[0]).is_some()
                            && obj_f64(&op.operands[1]).is_some()
                        {
                            show_ops = &op.operands[2..];
                        }
                    }
                    if op.operator == "'" || op.operator == "\"" {
                        let ty = -gs.leading;
                        apply_td(&mut ts, 0.0, ty);
                    }
                    let (pieces, adj) = text_pieces(show_ops);
                    self.emit_text(
                        page_index,
                        contents_id,
                        op_index as u32,
                        resource_owners,
                        &gs,
                        &mut ts,
                        &pieces,
                        adj,
                        nested,
                        geom_unsafe,
                    )?;
                }
                "Do" => {
                    let Some(name) = op.operands.first().and_then(|o| o.as_name().ok()) else {
                        continue;
                    };
                    let Some(id) = lookup_xobject(self.doc, resource_owners, name) else {
                        continue;
                    };
                    if xobject_is_form(self.doc, id) {
                        self.enter_form(
                            page_index,
                            id,
                            resource_owners,
                            &gs,
                            form_depth,
                            visiting,
                            geom_unsafe,
                        )?;
                    } else if xobject_is_image(self.doc, id) {
                        self.emit_image(
                            page_index,
                            contents_id,
                            op_index as u32,
                            id,
                            &gs,
                            nested,
                            geom_unsafe,
                        )?;
                    }
                }
                "BI" => {
                    self.emit_inline(
                        page_index,
                        contents_id,
                        op_index as u32,
                        &gs,
                        nested,
                        geom_unsafe,
                    )?;
                    inline_emitted += 1;
                }
                "EI" | "ID" => {}
                _ => {}
            }
        }

        while inline_emitted < inline_total {
            self.emit_inline(
                page_index,
                contents_id,
                ops.len() as u32 + inline_emitted as u32,
                &gs,
                nested,
                geom_unsafe,
            )?;
            inline_emitted += 1;
        }
        Ok(())
    }

    fn enter_form(
        &mut self,
        page_index: u32,
        form_id: ObjectId,
        resource_owners: &[ObjectId],
        gs: &GState,
        form_depth: usize,
        visiting: &mut Vec<ObjectId>,
        geom_unsafe: bool,
    ) -> Result<(), AppError> {
        if form_depth >= MAX_FORM_DEPTH {
            return Err(malformed_content(
                "Form XObject nesting is deeper than 8.",
            ));
        }
        if visiting.contains(&form_id) {
            return Err(malformed_content("A Form XObject refers to itself."));
        }
        let info = load_form(self.doc, form_id)?;
        self.add_decoded(info.bytes.len())?;
        let mut child_gs = gs.clone();
        child_gs.ctm = mul(gs.ctm, info.matrix);
        visiting.push(form_id);
        let mut child_owners = Vec::new();
        if info.has_resources {
            child_owners.push(form_id);
        }
        child_owners.extend_from_slice(resource_owners);
        let result = self.walk_stream(
            page_index,
            form_id,
            &info.bytes,
            &child_owners,
            child_gs,
            form_depth + 1,
            visiting,
            geom_unsafe,
        );
        visiting.pop();
        result
    }

    fn emit_text(
        &mut self,
        page_index: u32,
        contents_id: ObjectId,
        op_index: u32,
        resource_owners: &[ObjectId],
        gs: &GState,
        ts: &mut TextState,
        pieces: &[Vec<u8>],
        tj_adj: f64,
        nested: bool,
        geom_unsafe: bool,
    ) -> Result<(), AppError> {
        let inspect = inspect_text(
            self.doc,
            resource_owners,
            gs.font_name.as_deref(),
            pieces,
            tj_adj,
            gs.font_size,
        );
        let effective = mul(gs.ctm, ts.tm);
        let sx = (effective[0] * effective[0] + effective[1] * effective[1]).sqrt();
        let sy = (effective[2] * effective[2] + effective[3] * effective[3]).sqrt();
        let height = (gs.font_size.abs() * sy).max(0.01);
        let th = gs.hscale / 100.0;
        let shown = inspect.width * th;
        let width = (shown * sx).abs().max(0.01);
        let tx = (inspect.width + spacing_advance(pieces, gs.tc, gs.tw)) * th;
        let flags = Flags {
            nested_form: nested,
            type3: inspect.type3,
            vertical: inspect.vertical,
            clipped: gs.clipped(),
            pattern: gs.pattern(),
            rotated: is_rotated_tm(effective),
            skewed: is_skewed_tm(effective),
            no_tounicode: inspect.no_tounicode,
            missing_font: inspect.missing_font,
            ambiguous: inspect.ambiguous,
            masked: gs.masked,
            geometry: geom_unsafe,
            ..Flags::default()
        };
        self.push(Pending {
            page_index,
            kind: SourceKind::Text,
            rect: SourceRect {
                x: effective[4],
                y: effective[5],
                w: width,
                h: height,
            },
            locator: encode_locator(
                self.fp,
                page_index,
                SourceKind::Text,
                contents_id,
                op_index,
                inspect.font_id,
            ),
            flags,
            paint_id: None,
        })?;
        // Shown-width advance is Tm only; Tlm stays at the line origin.
        ts.tm = mul(ts.tm, [1.0, 0.0, 0.0, 1.0, tx, 0.0]);
        Ok(())
    }

    fn emit_image(
        &mut self,
        page_index: u32,
        contents_id: ObjectId,
        op_index: u32,
        image_id: ObjectId,
        gs: &GState,
        nested: bool,
        geom_unsafe: bool,
    ) -> Result<(), AppError> {
        let flags = Flags {
            nested_form: nested,
            clipped: gs.clipped(),
            pattern: gs.pattern(),
            masked: gs.masked || image_is_masked(self.doc, image_id),
            geometry: geom_unsafe,
            ..Flags::default()
        };
        self.push(Pending {
            page_index,
            kind: SourceKind::Image,
            rect: unit_square_bbox(gs.ctm),
            locator: encode_locator(
                self.fp,
                page_index,
                SourceKind::Image,
                contents_id,
                op_index,
                image_id,
            ),
            flags,
            paint_id: Some(image_id),
        })
    }

    fn emit_inline(
        &mut self,
        page_index: u32,
        contents_id: ObjectId,
        op_index: u32,
        gs: &GState,
        nested: bool,
        geom_unsafe: bool,
    ) -> Result<(), AppError> {
        let flags = Flags {
            inline_image: true,
            nested_form: nested,
            clipped: gs.clipped(),
            pattern: gs.pattern(),
            masked: gs.masked,
            geometry: geom_unsafe,
            ..Flags::default()
        };
        self.push(Pending {
            page_index,
            kind: SourceKind::Image,
            rect: unit_square_bbox(gs.ctm),
            locator: encode_locator(
                self.fp,
                page_index,
                SourceKind::Image,
                contents_id,
                op_index,
                (0, 0),
            ),
            flags,
            paint_id: None,
        })
    }

    fn add_decoded(&mut self, n: usize) -> Result<(), AppError> {
        self.decoded_total = self.decoded_total.saturating_add(n);
        if self.decoded_total > MAX_DECODED_TOTAL {
            return Err(malformed_content("Decoded content exceeds 64 MB."));
        }
        Ok(())
    }

    fn bump_ops(&mut self, n: usize) -> Result<(), AppError> {
        self.op_count = self.op_count.saturating_add(n);
        if self.op_count > MAX_OPS {
            return Err(malformed_content(
                "The page content has more than 50,000 operators.",
            ));
        }
        Ok(())
    }

    fn push(&mut self, mut pending: Pending) -> Result<(), AppError> {
        if self.pending.len() >= MAX_OCCURRENCES {
            return Err(malformed_content(
                "This PDF has more than 5,000 text or image occurrences.",
            ));
        }
        if let Some(id) = pending.paint_id {
            *self.paint_counts.entry(id).or_insert(0) += 1;
        }
        // A Form stream/operator can be visited more than once on the same
        // page. Include its deterministic occurrence ordinal, not just its ID.
        pending.locator.push_str(&format!(":{}", self.pending.len()));
        self.pending.push(pending);
        Ok(())
    }

    fn finish(self) -> Vec<SourceOccurrence> {
        let counts = self.paint_counts;
        self.pending
            .into_iter()
            .map(|mut p| {
                if let Some(id) = p.paint_id {
                    if counts.get(&id).copied().unwrap_or(0) > 1 {
                        p.flags.shared = true;
                    }
                }
                let (capability, reason) = pick_reason(&p.flags);
                SourceOccurrence {
                    page_index: p.page_index,
                    kind: p.kind,
                    rect: p.rect,
                    locator: p.locator,
                    capability,
                    reason,
                }
            })
            .collect()
    }
}

fn pick_reason(flags: &Flags) -> (SourceCapability, Option<String>) {
    let code = if flags.inline_image {
        Some("INLINE_IMAGE")
    } else if flags.nested_form {
        Some("NESTED_FORM")
    } else if flags.type3 {
        Some("TYPE3")
    } else if flags.vertical {
        Some("VERTICAL")
    } else if flags.clipped {
        Some("CLIPPED")
    } else if flags.pattern {
        Some("PATTERN")
    } else if flags.rotated {
        Some("ROTATED_TEXT")
    } else if flags.skewed {
        Some("SKEWED_TEXT")
    } else if flags.missing_font {
        Some("MISSING_FONT")
    } else if flags.no_tounicode {
        Some("NO_TOUNICODE")
    } else if flags.ambiguous {
        Some("AMBIGUOUS_UNICODE")
    } else if flags.masked {
        Some("MASKED_IMAGE")
    } else if flags.shared {
        Some("SHARED_XOBJECT")
    } else if flags.geometry {
        Some("GEOMETRY")
    } else {
        None
    };
    match code {
        Some(c) => (SourceCapability::Unsupported, Some(c.to_string())),
        None => (SourceCapability::Supported, None),
    }
}

fn page_geom_unsafe(doc: &Document, page_id: ObjectId) -> bool {
    if crop::page_rotation(doc, page_id) != 0 {
        return true;
    }
    if (crop::page_user_unit(doc, page_id) - 1.0).abs() > 1e-9 {
        return true;
    }
    let mb = crop::media_box(doc, page_id);
    if let Some(cb) = crop::crop_box(doc, page_id) {
        if (cb[0] - mb[0]).abs() > 1e-6 || (cb[1] - mb[1]).abs() > 1e-6 {
            return true;
        }
    }
    false
}

fn page_resource_owners(doc: &Document, page_id: ObjectId) -> Vec<ObjectId> {
    let mut out = Vec::new();
    let mut cur = Some(page_id);
    let mut steps = 0;
    while let Some(id) = cur {
        if steps > 32 {
            break;
        }
        steps += 1;
        let Some(dict) = object_dict(doc, id) else {
            break;
        };
        if dict.get(b"Resources").is_ok() {
            out.push(id);
        }
        cur = dict.get(b"Parent").ok().and_then(|o| o.as_reference().ok());
    }
    out
}

fn object_dict(doc: &Document, id: ObjectId) -> Option<&Dictionary> {
    match doc.get_object(id).ok()? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

fn resources_of<'a>(doc: &'a Document, owner: ObjectId) -> Option<&'a Dictionary> {
    match object_dict(doc, owner)?.get(b"Resources").ok()? {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn named_resource_entry<'a>(
    doc: &'a Document,
    owner: ObjectId,
    category: &[u8],
    name: &[u8],
) -> Option<&'a Object> {
    let res = resources_of(doc, owner)?;
    let cat = match res.get(category).ok()? {
        Object::Dictionary(d) => d,
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        _ => return None,
    };
    cat.get(name).ok()
}

fn lookup_xobject(doc: &Document, owners: &[ObjectId], name: &[u8]) -> Option<ObjectId> {
    for &owner in owners {
        if let Some(Object::Reference(id)) = named_resource_entry(doc, owner, b"XObject", name) {
            return Some(*id);
        }
    }
    None
}

enum FontRef<'a> {
    Id(ObjectId),
    Dict(&'a Dictionary),
}

fn lookup_font<'a>(doc: &'a Document, owners: &[ObjectId], name: &[u8]) -> Option<FontRef<'a>> {
    for &owner in owners {
        match named_resource_entry(doc, owner, b"Font", name) {
            Some(Object::Reference(id)) => return Some(FontRef::Id(*id)),
            Some(Object::Dictionary(d)) => return Some(FontRef::Dict(d)),
            _ => {}
        }
    }
    None
}

fn font_dict<'a>(doc: &'a Document, font: &FontRef<'a>) -> Option<&'a Dictionary> {
    match font {
        FontRef::Id(id) => match doc.get_object(*id).ok()? {
            Object::Dictionary(d) => Some(d),
            Object::Stream(s) => Some(&s.dict),
            _ => None,
        },
        FontRef::Dict(d) => Some(*d),
    }
}

fn xobject_subtype<'a>(doc: &'a Document, id: ObjectId) -> Option<&'a [u8]> {
    object_dict(doc, id)?
        .get(b"Subtype")
        .ok()?
        .as_name()
        .ok()
}

fn xobject_is_form(doc: &Document, id: ObjectId) -> bool {
    xobject_subtype(doc, id) == Some(b"Form")
}

fn xobject_is_image(doc: &Document, id: ObjectId) -> bool {
    xobject_subtype(doc, id) == Some(b"Image")
}

fn image_is_masked(doc: &Document, id: ObjectId) -> bool {
    let Some(dict) = object_dict(doc, id) else {
        return false;
    };
    if dict.get(b"Mask").is_ok() || dict.get(b"SMask").is_ok() {
        return true;
    }
    match dict.get(b"ImageMask") {
        Ok(Object::Boolean(true)) => true,
        Ok(Object::Integer(i)) if *i != 0 => true,
        _ => false,
    }
}

fn load_form(doc: &Document, id: ObjectId) -> Result<FormInfo, AppError> {
    let stream = doc
        .get_object(id)
        .ok()
        .and_then(|o| o.as_stream().ok())
        .ok_or_else(|| malformed_content("A Form XObject is unreadable."))?;
    Ok(FormInfo {
        matrix: matrix_from_dict(&stream.dict),
        has_resources: stream.dict.get(b"Resources").is_ok(),
        bytes: decompress_stream(stream)?,
    })
}

fn matrix_from_dict(dict: &Dictionary) -> [f64; 6] {
    let Ok(obj) = dict.get(b"Matrix") else {
        return IDENTITY;
    };
    let arr = match obj {
        Object::Array(a) => a,
        _ => return IDENTITY,
    };
    if arr.len() < 6 {
        return IDENTITY;
    }
    let mut m = IDENTITY;
    for (i, item) in arr.iter().take(6).enumerate() {
        match obj_f64(item) {
            Some(n) => m[i] = n,
            None => return IDENTITY,
        }
    }
    m
}

fn decompress_stream(stream: &Stream) -> Result<Vec<u8>, AppError> {
    let data = match stream.decompressed_content() {
        Ok(d) => d,
        Err(_) => stream.content.clone(),
    };
    if data.len() > MAX_STREAM_BYTES {
        return Err(malformed_content(
            "A content stream is larger than 32 MB decompressed.",
        ));
    }
    Ok(data)
}

fn inspect_text(
    doc: &Document,
    owners: &[ObjectId],
    font_name: Option<&[u8]>,
    pieces: &[Vec<u8>],
    tj_adj: f64,
    font_size: f64,
) -> TextInspect {
    let font_ref = font_name.and_then(|n| lookup_font(doc, owners, n));
    let font_id = match &font_ref {
        Some(FontRef::Id(id)) => *id,
        _ => (0, 0),
    };
    let dict = font_ref.as_ref().and_then(|f| font_dict(doc, f));
    let mut type3 = false;
    let mut vertical = false;
    let mut no_tounicode = false;
    let mut ambiguous = false;
    let mut width_sum = 0.0;
    if let Some(font) = dict {
        type3 = is_type3(font);
        vertical = is_vertical(doc, font);
        if is_cid_or_type0(font) {
            if tounicode_usable(doc, font) {
                ambiguous = true;
            } else {
                no_tounicode = true;
            }
        }
        for piece in pieces {
            width_sum += glyph_width_sum(doc, font, piece);
        }
    } else {
        for piece in pieces {
            width_sum += piece.iter().map(|&b| helvetica_width(b)).sum::<f64>();
        }
    }
    let size = font_size.abs().max(0.01);
    TextInspect {
        missing_font: dict.is_none(),
        type3,
        vertical,
        no_tounicode,
        ambiguous,
        width: (width_sum + tj_adj) / 1000.0 * size,
        font_id,
    }
}

fn is_type3(font: &Dictionary) -> bool {
    font.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) == Some(b"Type3")
        || font.get(b"CharProcs").is_ok()
}

fn is_cid_or_type0(font: &Dictionary) -> bool {
    match font.get(b"Subtype").ok().and_then(|o| o.as_name().ok()) {
        Some(b"Type0") | Some(b"CIDFontType0") | Some(b"CIDFontType2") => return true,
        _ => {}
    }
    if font.get(b"CIDSystemInfo").is_ok() || font.get(b"DescendantFonts").is_ok() {
        return true;
    }
    matches!(
        font.get(b"Encoding").ok().and_then(|o| o.as_name().ok()),
        Some(b"Identity-H") | Some(b"Identity-V")
    )
}

fn is_vertical(doc: &Document, font: &Dictionary) -> bool {
    if font.get(b"Encoding").ok().and_then(|o| o.as_name().ok()) == Some(b"Identity-V") {
        return true;
    }
    if wmode_is_1(font) {
        return true;
    }
    descendant_is_vertical(doc, font)
}

fn wmode_is_1(font: &Dictionary) -> bool {
    matches!(font.get(b"WMode").ok().and_then(obj_f64), Some(w) if (w - 1.0).abs() < 0.5)
}

fn descendant_is_vertical(doc: &Document, font: &Dictionary) -> bool {
    let items = descendant_font_objects(doc, font);
    for item in &items {
        let dict = match item {
            Object::Dictionary(d) => d,
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(Object::Dictionary(d)) => d,
                Ok(Object::Stream(s)) => &s.dict,
                _ => continue,
            },
            _ => continue,
        };
        if dict.get(b"Encoding").ok().and_then(|o| o.as_name().ok()) == Some(b"Identity-V")
            || wmode_is_1(dict)
        {
            return true;
        }
    }
    false
}

fn descendant_font_objects(doc: &Document, font: &Dictionary) -> Vec<Object> {
    match font.get(b"DescendantFonts") {
        Ok(Object::Array(a)) => a.clone(),
        Ok(Object::Reference(id)) => match doc.get_object(*id) {
            Ok(Object::Array(a)) => a.clone(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn tounicode_usable(doc: &Document, font: &Dictionary) -> bool {
    let Ok(obj) = font.get(b"ToUnicode") else {
        return false;
    };
    let bytes = match obj {
        Object::Stream(s) => s.get_plain_content().unwrap_or_else(|_| s.content.clone()),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Stream(s)) => s.get_plain_content().unwrap_or_else(|_| s.content.clone()),
            _ => return false,
        },
        _ => return false,
    };
    let text = String::from_utf8_lossy(&bytes);
    text.contains("begincmap")
        && (text.contains("beginbfchar") || text.contains("beginbfrange"))
}

fn glyph_width_sum(doc: &Document, font: &Dictionary, bytes: &[u8]) -> f64 {
    if is_cid_or_type0(font) {
        return (bytes.len() / 2) as f64 * descendant_dw(doc, font);
    }
    if let Some(widths) = explicit_widths(doc, font) {
        return bytes.iter().map(|&b| widths[b as usize]).sum();
    }
    bytes.iter().map(|&b| helvetica_width(b)).sum()
}

fn descendant_dw(doc: &Document, font: &Dictionary) -> f64 {
    if let Some(n) = font.get(b"DW").ok().and_then(obj_f64) {
        return n;
    }
    for item in descendant_font_objects(doc, font) {
        let dict = match &item {
            Object::Dictionary(d) => d,
            Object::Reference(id) => match doc.get_object(*id) {
                Ok(Object::Dictionary(d)) => d,
                _ => continue,
            },
            _ => continue,
        };
        if let Some(n) = dict.get(b"DW").ok().and_then(obj_f64) {
            return n;
        }
    }
    500.0
}

fn explicit_widths(doc: &Document, font: &Dictionary) -> Option<[f64; 256]> {
    // Helvetica table only when /Widths is truly absent (Standard-14).
    let arr = font.get_deref(b"Widths", doc).ok()?.as_array().ok()?;
    let first = font
        .get_deref(b"FirstChar", doc)
        .ok()
        .and_then(|o| o.as_i64().ok())
        .unwrap_or(0)
        .max(0) as usize;
    let last = font
        .get_deref(b"LastChar", doc)
        .ok()
        .and_then(|o| o.as_i64().ok())
        .map(|n| n.max(0) as usize)
        .unwrap_or_else(|| first.saturating_add(arr.len().saturating_sub(1)).min(255));
    let mut widths = [500.0; 256];
    for (i, obj) in arr.iter().enumerate() {
        let code = first + i;
        if code > last || code > 255 {
            break;
        }
        if let Some(n) = obj_f64(obj) {
            widths[code] = n;
        }
    }
    Some(widths)
}

/// Standard-14 Helvetica widths per 1000. Locked: H=667, i=278.
fn helvetica_width(code: u8) -> f64 {
    match code {
        b' ' => 278.0,
        b'!' => 278.0,
        b'"' => 355.0,
        b'#' => 556.0,
        b'$' => 556.0,
        b'%' => 889.0,
        b'&' => 667.0,
        b'\'' => 191.0,
        b'(' => 333.0,
        b')' => 333.0,
        b'*' => 389.0,
        b'+' => 584.0,
        b',' => 278.0,
        b'-' => 333.0,
        b'.' => 278.0,
        b'/' => 278.0,
        b'0'..=b'9' => 556.0,
        b':' => 278.0,
        b';' => 278.0,
        b'<' => 584.0,
        b'=' => 584.0,
        b'>' => 584.0,
        b'?' => 556.0,
        b'@' => 1015.0,
        b'A' => 667.0,
        b'B' => 667.0,
        b'C' => 722.0,
        b'D' => 722.0,
        b'E' => 667.0,
        b'F' => 611.0,
        b'G' => 778.0,
        b'H' => 667.0,
        b'I' => 278.0,
        b'J' => 500.0,
        b'K' => 667.0,
        b'L' => 556.0,
        b'M' => 833.0,
        b'N' => 722.0,
        b'O' => 778.0,
        b'P' => 667.0,
        b'Q' => 778.0,
        b'R' => 722.0,
        b'S' => 667.0,
        b'T' => 611.0,
        b'U' => 722.0,
        b'V' => 667.0,
        b'W' => 944.0,
        b'X' => 667.0,
        b'Y' => 667.0,
        b'Z' => 611.0,
        b'[' => 278.0,
        b'\\' => 278.0,
        b']' => 278.0,
        b'^' => 469.0,
        b'_' => 556.0,
        b'`' => 333.0,
        b'a' => 556.0,
        b'b' => 556.0,
        b'c' => 500.0,
        b'd' => 556.0,
        b'e' => 556.0,
        b'f' => 278.0,
        b'g' => 556.0,
        b'h' => 556.0,
        b'i' => 278.0,
        b'j' => 222.0,
        b'k' => 500.0,
        b'l' => 278.0,
        b'm' => 833.0,
        b'n' => 556.0,
        b'o' => 556.0,
        b'p' => 556.0,
        b'q' => 556.0,
        b'r' => 333.0,
        b's' => 500.0,
        b't' => 278.0,
        b'u' => 556.0,
        b'v' => 500.0,
        b'w' => 722.0,
        b'x' => 500.0,
        b'y' => 500.0,
        b'z' => 500.0,
        b'{' => 334.0,
        b'|' => 260.0,
        b'}' => 334.0,
        b'~' => 584.0,
        _ => 556.0,
    }
}

fn is_rotated_tm(m: [f64; 6]) -> bool {
    let [a, b, c, d, _, _] = m;
    let det = a * d - b * c;
    if det.abs() < 1e-6 {
        return false;
    }
    // 90° / 270°: off-axis, a≈d, b≈-c.
    if (b.abs() > 0.1 || c.abs() > 0.1) && (a - d).abs() < 0.25 && (b + c).abs() < 0.25 {
        return true;
    }
    // 180°: axis-aligned invert.
    a < 0.0 && d < 0.0 && b.abs() <= 0.05 && c.abs() <= 0.05
}

fn is_skewed_tm(m: [f64; 6]) -> bool {
    let [a, b, c, d, _, _] = m;
    if b.abs() < 0.05 && c.abs() < 0.05 {
        return false;
    }
    let det = a * d - b * c;
    if det.abs() < 1e-6 {
        return false;
    }
    !is_rotated_tm(m)
}

fn mul(m: [f64; 6], n: [f64; 6]) -> [f64; 6] {
    let [a, b, c, d, e, f] = m;
    let [a2, b2, c2, d2, e2, f2] = n;
    [
        a * a2 + c * b2,
        b * a2 + d * b2,
        a * c2 + c * d2,
        b * c2 + d * d2,
        a * e2 + c * f2 + e,
        b * e2 + d * f2 + f,
    ]
}

fn apply_point(m: [f64; 6], x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

fn unit_square_bbox(ctm: [f64; 6]) -> SourceRect {
    let pts = [
        apply_point(ctm, 0.0, 0.0),
        apply_point(ctm, 1.0, 0.0),
        apply_point(ctm, 0.0, 1.0),
        apply_point(ctm, 1.0, 1.0),
    ];
    let min_x = pts.iter().map(|p| p.0).fold(f64::INFINITY, f64::min);
    let min_y = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
    let max_x = pts.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max);
    let max_y = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
    SourceRect {
        x: min_x,
        y: min_y,
        w: (max_x - min_x).max(0.01),
        h: (max_y - min_y).max(0.01),
    }
}

fn apply_td(ts: &mut TextState, tx: f64, ty: f64) {
    let m = mul(ts.tlm, [1.0, 0.0, 0.0, 1.0, tx, ty]);
    ts.tm = m;
    ts.tlm = m;
}

fn obj_f64(obj: &Object) -> Option<f64> {
    match obj {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn six_nums(operands: &[Object]) -> Option<[f64; 6]> {
    if operands.len() < 6 {
        return None;
    }
    let start = operands.len() - 6;
    let mut m = [0.0; 6];
    for i in 0..6 {
        m[i] = obj_f64(&operands[start + i])?;
    }
    Some(m)
}

fn two_nums(operands: &[Object]) -> Option<(f64, f64)> {
    if operands.len() < 2 {
        return None;
    }
    let n = operands.len();
    Some((obj_f64(&operands[n - 2])?, obj_f64(&operands[n - 1])?))
}

fn is_pattern_name(operands: &[Object]) -> bool {
    operands
        .iter()
        .any(|o| o.as_name().ok() == Some(b"Pattern"))
}

fn operand_selects_pattern_space(
    doc: &Document,
    owners: &[ObjectId],
    operands: &[Object],
) -> bool {
    let Some(obj) = operands.first() else {
        return false;
    };
    if color_space_object_is_pattern(doc, obj) {
        return true;
    }
    let Ok(name) = obj.as_name() else {
        return false;
    };
    if name == b"Pattern" {
        return true;
    }
    for &owner in owners {
        if let Some(entry) = named_resource_entry(doc, owner, b"ColorSpace", name) {
            return color_space_object_is_pattern(doc, entry);
        }
    }
    false
}

fn color_space_object_is_pattern(doc: &Document, obj: &Object) -> bool {
    let resolved = match doc.dereference(obj) {
        Ok((_, o)) => o,
        Err(_) => return false,
    };
    match resolved {
        Object::Name(n) => n.as_slice() == b"Pattern",
        Object::Array(arr) => arr.first().is_some_and(|first| {
            let first = doc.dereference(first).map(|(_, o)| o).unwrap_or(first);
            first.as_name().ok() == Some(b"Pattern")
        }),
        _ => false,
    }
}

fn apply_extgstate_smask(doc: &Document, owners: &[ObjectId], name: &[u8], gs: &mut GState) {
    for &owner in owners {
        let Some(entry) = named_resource_entry(doc, owner, b"ExtGState", name) else {
            continue;
        };
        let Some(dict) = deref_dict(doc, entry) else {
            continue;
        };
        let Ok(smask) = dict.get(b"SMask") else {
            return;
        };
        let resolved = doc.dereference(smask).map(|(_, o)| o).unwrap_or(smask);
        gs.masked = resolved.as_name().ok() != Some(b"None");
        return;
    }
}

fn deref_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match doc.dereference(obj) {
        Ok((_, Object::Dictionary(d))) => Some(d),
        Ok((_, Object::Stream(s))) => Some(&s.dict),
        _ => None,
    }
}

fn text_pieces(operands: &[Object]) -> (Vec<Vec<u8>>, f64) {
    let mut pieces = Vec::new();
    let mut adj = 0.0;
    for obj in operands {
        collect_text_obj(obj, &mut pieces, &mut adj);
    }
    (pieces, adj)
}

fn collect_text_obj(obj: &Object, pieces: &mut Vec<Vec<u8>>, adj: &mut f64) {
    match obj {
        Object::String(s, _) => pieces.push(s.clone()),
        Object::Integer(i) => *adj -= *i as f64,
        Object::Real(r) => *adj -= *r as f64,
        Object::Array(arr) => {
            for item in arr {
                collect_text_obj(item, pieces, adj);
            }
        }
        _ => {}
    }
}

fn spacing_advance(pieces: &[Vec<u8>], tc: f64, tw: f64) -> f64 {
    let mut extra = 0.0;
    for piece in pieces {
        extra += piece.len() as f64 * tc;
        extra += piece.iter().filter(|&&b| b == b' ').count() as f64 * tw;
    }
    extra
}

fn strip_inline_image_payloads(data: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    // Normal → Keys (after BI) → Payload (after ID). Copy BI/keys/ID/EI;
    // drop only the raw bytes between ID and EI so Content::decode still
    // yields BI at the CTM in force there.
    let mut after_bi = false;
    let mut after_id = false;
    while i < data.len() {
        if after_id {
            if is_op_token(data, i, b"EI") {
                out.extend_from_slice(b"EI");
                i += 2;
                after_id = false;
                after_bi = false;
            } else {
                i += 1;
            }
            continue;
        }
        if data[i].is_ascii_whitespace() {
            out.push(data[i]);
            i += 1;
            continue;
        }
        if data[i] == b'%' {
            let start = i;
            while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            out.extend_from_slice(&data[start..i]);
            continue;
        }
        if data[i] == b'(' {
            let start = i;
            i = skip_literal(data, i)?;
            out.extend_from_slice(&data[start..i]);
            continue;
        }
        if data[i] == b'<' && data.get(i + 1) != Some(&b'<') {
            let start = i;
            i = skip_hex(data, i);
            out.extend_from_slice(&data[start..i]);
            continue;
        }
        if !after_bi && is_op_token(data, i, b"BI") {
            out.extend_from_slice(b"BI");
            i += 2;
            after_bi = true;
            continue;
        }
        if after_bi && is_op_token(data, i, b"ID") {
            out.extend_from_slice(b"ID");
            i += 2;
            out.push(b' ');
            after_id = true;
            continue;
        }
        if after_bi && is_op_token(data, i, b"EI") {
            out.extend_from_slice(b"EI");
            i += 2;
            after_bi = false;
            continue;
        }
        out.push(data[i]);
        i += 1;
    }
    if after_bi || after_id {
        return Err(malformed_content("An inline image is unterminated."));
    }
    Ok(out)
}

fn count_inline_images(data: &[u8]) -> Result<usize, AppError> {
    let mut count = 0;
    let mut i = 0;
    let mut in_inline = false;
    while i < data.len() {
        if data[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if !in_inline && data[i] == b'%' {
            while i < data.len() && data[i] != b'\n' && data[i] != b'\r' {
                i += 1;
            }
            continue;
        }
        if !in_inline && data[i] == b'(' {
            i = skip_literal(data, i)?;
            continue;
        }
        if !in_inline && data[i] == b'<' && data.get(i + 1) != Some(&b'<') {
            i = skip_hex(data, i);
            continue;
        }
        if !in_inline && is_op_token(data, i, b"BI") {
            in_inline = true;
            i += 2;
            continue;
        }
        if in_inline && is_op_token(data, i, b"EI") {
            count += 1;
            in_inline = false;
            i += 2;
            continue;
        }
        i += 1;
    }
    if in_inline {
        return Err(malformed_content("An inline image is unterminated."));
    }
    Ok(count)
}

fn is_delim(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
        )
}

fn is_op_token(data: &[u8], i: usize, token: &[u8]) -> bool {
    if i + token.len() > data.len() || &data[i..i + token.len()] != token {
        return false;
    }
    let before = i == 0 || is_delim(data[i - 1]);
    let after = i + token.len() == data.len() || is_delim(data[i + token.len()]);
    before && after
}

fn skip_literal(data: &[u8], start: usize) -> Result<usize, AppError> {
    let mut i = start + 1;
    let mut depth = 1;
    while i < data.len() && depth > 0 {
        match data[i] {
            b'\\' => {
                i = i.saturating_add(2).min(data.len());
                continue;
            }
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if depth != 0 {
        return Err(malformed_content("A literal string is unterminated."));
    }
    Ok(i)
}

fn skip_hex(data: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < data.len() && data[i] != b'>' {
        i += 1;
    }
    if i < data.len() {
        i + 1
    } else {
        i
    }
}

fn document_is_signed(doc: &Document) -> bool {
    if let Ok(cat) = doc.catalog() {
        if cat.get(b"Perms").is_ok() {
            return true;
        }
    }
    for obj in doc.objects.values() {
        if object_is_signature(obj) {
            return true;
        }
    }
    false
}

fn object_is_signature(obj: &Object) -> bool {
    let dict = match obj {
        Object::Dictionary(d) => d,
        Object::Stream(s) => &s.dict,
        _ => return false,
    };
    let is_sig = name_is(dict, b"Type", b"Sig") || name_is(dict, b"FT", b"Sig");
    is_sig && dict.get(b"ByteRange").is_ok()
}

fn name_is(dict: &Dictionary, key: &[u8], expect: &[u8]) -> bool {
    dict.get(key).ok().and_then(|o| o.as_name().ok()) == Some(expect)
}

fn fnv1a_u64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn encode_locator(
    fp: u64,
    page_index: u32,
    kind: SourceKind,
    contents_id: ObjectId,
    op_index: u32,
    object_id: ObjectId,
) -> String {
    let k = match kind {
        SourceKind::Text => 0u8,
        SourceKind::Image => 1u8,
    };
    format!(
        "v1:{fp:016x}:{page_index}:{k}:{}:{}:{op_index}:{}:{}",
        contents_id.0, contents_id.1, object_id.0, object_id.1
    )
}

fn parse_locator_fp(locator: &str) -> Option<u64> {
    let rest = locator.strip_prefix("v1:")?;
    let hex = rest.split(':').next()?;
    u64::from_str_radix(hex, 16).ok()
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn file_too_large() -> AppError {
    AppError::new(
        "FILE_TOO_LARGE",
        "File too large to classify",
        "Classifying source content needs the document loaded into memory, and this file is over 400 MB.",
    )
    .with_suggestion("Use a smaller PDF.")
}

fn malformed_content(message: impl Into<String>) -> AppError {
    AppError::new(
        "MALFORMED_CONTENT",
        "This PDF content cannot be read",
        message,
    )
    .with_suggestion("Open the file in a PDF editor that can repair it, or use a different PDF.")
}

fn encrypted() -> AppError {
    AppError::new(
        "ENCRYPTED",
        "This PDF is encrypted",
        "OffPDF cannot classify source content in an encrypted PDF.",
    )
    .with_suggestion("Unlock the PDF and try again.")
}

fn signed() -> AppError {
    AppError::new(
        "SIGNED",
        "This PDF is signed",
        "OffPDF cannot classify source content in a signed PDF.",
    )
    .with_suggestion("Use an unsigned copy of the file.")
}

fn stale() -> AppError {
    AppError::new(
        "STALE",
        "This locator is stale",
        "The source file no longer matches the locator fingerprint.",
    )
}
