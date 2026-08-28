//! Detect and fill existing AcroForm fields (`list_form_fields` / `apply_form_values`).
//!
//! One lopdf walker lists terminals and writes `/V` plus a hand-rolled `/AP`
//! (bundled Noto, Identity-H). Flatten is widgets only. qpdf is the `#34`
//! check / optional xref rewrite — not the form writer.

use crate::error::AppError;
use crate::pdf_engine::crop;
use crate::pdf_engine::metadata::{decode_pdf_string, encode_pdf_string};
use crate::pdf_engine::qpdf;
use crate::pdf_engine::validate_output::{
    catalog_flags_from_doc, content_digest, validate_staged_pdf, OutputSnapshot, PageSnapshot,
};
use crate::utils::safe_output;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use ttf_parser::{Face, GlyphId};

/// lopdf loads the whole file; refuse enormous inputs instead of risking RAM.
const MAX_FORM_BYTES: u64 = 400 * 1024 * 1024;
const MAX_WALK_DEPTH: usize = 32;
const MAX_WALK_NODES: usize = 8_000;

const FF_READONLY: u32 = 1 << 0;
const FF_MULTILINE: u32 = 1 << 12;
const FF_RADIO: u32 = 1 << 15;
const FF_PUSHBUTTON: u32 = 1 << 16;
const FF_COMBO: u32 = 1 << 17;
const FF_EDIT: u32 = 1 << 18;
const F_INVISIBLE: i64 = 1 << 0;
const F_HIDDEN: i64 = 1 << 1;
const F_NOVIEW: i64 = 1 << 5;

/// Fillable terminal kinds. Pushbuttons and `/FT /Sig` are not listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FormFieldKind {
    Text,
    Checkbox,
    Radio,
    Combo,
    List,
}

/// Unrotated PDF user space — same contract as `EditObject.rect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// One fillable control from `list_form_fields`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormField {
    pub name: String,
    pub kind: FormFieldKind,
    /// Assembled 0-based page index when the widget has a page.
    pub page_index: Option<u32>,
    /// Widget `/Rect` as `{x: llx, y: lly, w, h}` in unrotated user space.
    pub rect: Option<FormRect>,
    pub value: Option<String>,
    /// Checkbox/radio on-state names from `/AP /N` other than `/Off`.
    pub export_values: Vec<String>,
    /// Combo/list `/Opt` choices.
    pub choices: Vec<String>,
    pub read_only: bool,
    pub hidden: bool,
    pub max_len: Option<u32>,
    pub multiline: bool,
    pub combo_edit: bool,
}

/// Session payload: field full name → value. Never PDF bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormValue {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Default)]
struct Inherited {
    ft: Option<Vec<u8>>,
    ff: Option<u32>,
    v: Option<Object>,
    da: Option<Object>,
    q: Option<i64>,
    opt: Option<Object>,
    max_len: Option<u32>,
}

struct WalkedField {
    name: String,
    kind: FormFieldKind,
    /// Object that holds `/V` (parent for radio; the terminal otherwise).
    value_id: Option<ObjectId>,
    /// Inline `/V` dict sitting in `/Fields` (no object id).
    inline_fields_index: Option<usize>,
    widget_ids: Vec<ObjectId>,
    page_index: Option<u32>,
    rect: Option<FormRect>,
    value: Option<String>,
    export_values: Vec<String>,
    choices: Vec<String>,
    read_only: bool,
    hidden: bool,
    max_len: Option<u32>,
    multiline: bool,
    combo_edit: bool,
}

struct WalkState {
    out: Vec<WalkedField>,
    visiting: HashSet<ObjectId>,
    seen: HashSet<ObjectId>,
    nodes: usize,
}

impl WalkState {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            visiting: HashSet::new(),
            seen: HashSet::new(),
            nodes: 0,
        }
    }

    fn touch(&mut self) -> Result<(), AppError> {
        self.nodes += 1;
        if self.nodes > MAX_WALK_NODES {
            return Err(malformed("This form has too many fields to walk safely."));
        }
        Ok(())
    }
}

/// Classify a resolved `/FT` + `/Ff`. Pushbuttons and `/FT /Sig` are skipped.
pub fn classify_field(ft: Option<&[u8]>, ff: u32) -> Option<FormFieldKind> {
    match ft {
        Some(b"Tx") => Some(FormFieldKind::Text),
        Some(b"Btn") => {
            if ff & FF_PUSHBUTTON != 0 {
                None
            } else if ff & FF_RADIO != 0 {
                Some(FormFieldKind::Radio)
            } else {
                Some(FormFieldKind::Checkbox)
            }
        }
        Some(b"Ch") => {
            if ff & FF_COMBO != 0 {
                Some(FormFieldKind::Combo)
            } else {
                Some(FormFieldKind::List)
            }
        }
        Some(b"Sig") | None => None,
        _ => None,
    }
}

/// `/XFA` (stream or array) or `/NeedsRendering` → `UNSUPPORTED_XFA`.
pub fn detect_xfa(path: &str) -> Result<(), AppError> {
    super::require_input(path)?;
    size_gate(path)?;
    let doc = load_doc(path)?;
    detect_xfa_doc(&doc)
}

pub fn list_form_fields(path: &str) -> Result<Vec<FormField>, AppError> {
    super::require_input(path)?;
    size_gate(path)?;
    let doc = load_doc(path)?;
    detect_xfa_doc(&doc)?;
    let walked = walk_fields(&doc)?;
    Ok(walked
        .into_iter()
        .map(|f| FormField {
            name: f.name,
            kind: f.kind,
            page_index: f.page_index,
            rect: f.rect,
            value: f.value,
            export_values: f.export_values,
            choices: f.choices,
            read_only: f.read_only,
            hidden: f.hidden,
            max_len: f.max_len,
            multiline: f.multiline,
            combo_edit: f.combo_edit,
        })
        .collect())
}

pub fn apply_form_values(
    path: &str,
    values: &[FormValue],
    flatten: bool,
) -> Result<(), AppError> {
    super::require_input(path)?;
    size_gate(path)?;
    let mut doc = load_doc(path)?;
    detect_xfa_doc(&doc)?;
    apply_form_values_doc(&mut doc, values, flatten)?;
    doc.save(path)
        .map_err(|e| AppError::io("Could not write the filled PDF.", e))?;
    Ok(())
}

/// F14 save predicate: stamps **or** form values are an edit.
pub fn has_edits(stamp_count: usize, form_values: &[FormValue]) -> bool {
    stamp_count > 0 || !form_values.is_empty()
}

/// Form-path publish (staged tmp → `#34` → dest). Original file is never overwritten.
pub fn fill_pdf_form(
    input: &str,
    output: &str,
    values: &[FormValue],
    flatten: bool,
) -> Result<(), AppError> {
    super::require_input(input)?;
    super::ensure_output_dir(output)?;
    size_gate(input)?;
    if safe_output::same_file_identity(Path::new(input), Path::new(output)) {
        return Err(AppError::new(
            "OVERWRITE",
            "Choose a new file name",
            "OffPDF never overwrites the original PDF.",
        )
        .with_suggestion("Pick a different name or folder."));
    }

    let dest = Path::new(output);
    let unique = format!(
        "form-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let tmp = safe_output::sibling_temp_path(dest, &unique)?;
    let result = (|| -> Result<(), AppError> {
        std::fs::copy(input, &tmp).map_err(|e| AppError::io("Could not stage the PDF.", e))?;
        apply_form_values(&tmp.to_string_lossy(), values, flatten)?;
        let cleaned = tmp.with_file_name(format!(
            "{}.qpdf.pdf",
            tmp.file_name()
                .map(|s| s.to_string_lossy())
                .unwrap_or_default()
        ));
        qpdf_rewrite(&tmp, &cleaned)?;
        let _ = std::fs::remove_file(&tmp);
        std::fs::rename(&cleaned, &tmp)
            .map_err(|e| AppError::io("Could not stage the filled PDF.", e))?;
        let snapshot = snapshot_for_form_dest(input, &tmp, flatten)?;
        let exe = qpdf::resolve_qpdf_standalone();
        validate_staged_pdf(&tmp, &snapshot, None, |args| run_qpdf_check(&exe, args))?;
        safe_output::replace_file(&tmp, dest)?;
        Ok(())
    })();
    if tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Extra workspace files that carry their own form tree cannot be filled.
pub fn extra_files_have_fields(paths: &[&str]) -> Result<(), AppError> {
    for path in paths {
        match list_form_fields(path) {
            Ok(fields) if !fields.is_empty() => {
                return Err(AppError::new(
                    "EXTRA_FILE_HAS_FORM",
                    "Only the first PDF's form can be filled",
                    format!(
                        "\"{path}\" also has form fields. OffPDF fills the primary file only."
                    ),
                )
                .with_suggestion(
                    "Remove extra files that have forms, or fill each PDF on its own.",
                ));
            }
            Err(e) if e.code == "UNSUPPORTED_XFA" || e.code == "MALFORMED_FORM" => {
                return Err(e);
            }
            _ => {}
        }
    }
    Ok(())
}

fn size_gate(input: &str) -> Result<(), AppError> {
    let meta = std::fs::metadata(input).map_err(|e| AppError::io("Could not read the file.", e))?;
    if meta.len() > MAX_FORM_BYTES {
        return Err(AppError::new(
            "FILE_TOO_LARGE",
            "File too large for form editing",
            "Filling a form needs the document loaded into memory, and this file is over 400 MB.",
        )
        .with_suggestion("Use a smaller PDF."));
    }
    Ok(())
}

fn load_doc(path: &str) -> Result<Document, AppError> {
    Document::load(path).map_err(|e| AppError::invalid_pdf(path).with_details(format!("lopdf: {e}")))
}

fn malformed(message: impl Into<String>) -> AppError {
    AppError::new(
        "MALFORMED_FORM",
        "This PDF form cannot be read",
        message,
    )
    .with_suggestion("Open the file in a PDF editor that can repair forms, or use a different PDF.")
}

fn unsupported_xfa() -> AppError {
    AppError::new(
        "UNSUPPORTED_XFA",
        "This PDF uses an XFA form",
        "OffPDF can fill AcroForm fields, but this file uses the older XFA form format.",
    )
    .with_suggestion("Export or save the PDF as an AcroForm (not XFA) and try again.")
}

fn detect_xfa_doc(doc: &Document) -> Result<(), AppError> {
    let Some(acro) = acroform_dict(doc) else {
        return Ok(());
    };
    if acro.get(b"XFA").is_ok() {
        return Err(unsupported_xfa());
    }
    match acro.get(b"NeedsRendering") {
        Ok(Object::Boolean(true)) => return Err(unsupported_xfa()),
        Ok(Object::Integer(i)) if *i != 0 => return Err(unsupported_xfa()),
        _ => {}
    }
    Ok(())
}

fn acroform_dict(doc: &Document) -> Option<&Dictionary> {
    let root = doc.trailer.get(b"Root").ok()?.as_reference().ok()?;
    let cat = doc.get_dictionary(root).ok()?;
    match cat.get(b"AcroForm").ok()? {
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        Object::Dictionary(d) => Some(d),
        _ => None,
    }
}

fn acroform_fields<'a>(doc: &'a Document) -> Result<Option<&'a [Object]>, AppError> {
    let Some(acro) = acroform_dict(doc) else {
        return Ok(None);
    };
    match acro.get(b"Fields") {
        Ok(Object::Array(a)) => Ok(Some(a)),
        Ok(_) => Err(malformed("The form /Fields entry is not an array.")),
        Err(_) => Ok(Some(&[])),
    }
}

fn walk_fields(doc: &Document) -> Result<Vec<WalkedField>, AppError> {
    let Some(fields) = acroform_fields(doc)? else {
        return Ok(Vec::new());
    };
    let pages = page_index_map(doc);
    let mut state = WalkState::new();
    for (i, obj) in fields.iter().enumerate() {
        walk_node(
            doc,
            obj,
            &Inherited::default(),
            "",
            0,
            Some(i),
            &pages,
            &mut state,
        )?;
    }
    Ok(state.out)
}

fn page_index_map(doc: &Document) -> BTreeMap<ObjectId, u32> {
    doc.get_pages()
        .into_iter()
        .map(|(n, id)| (id, n.saturating_sub(1)))
        .collect()
}

fn walk_node(
    doc: &Document,
    obj: &Object,
    inherited: &Inherited,
    parent_name: &str,
    depth: usize,
    inline_fields_index: Option<usize>,
    pages: &BTreeMap<ObjectId, u32>,
    state: &mut WalkState,
) -> Result<(), AppError> {
    if depth > MAX_WALK_DEPTH {
        return Err(malformed("This form's field tree is nested too deeply."));
    }
    state.touch()?;

    let (dict, id) = match resolve_dict(doc, obj) {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

    if let Some(id) = id {
        if state.visiting.contains(&id) {
            return Err(malformed("This form's field tree refers back to itself."));
        }
        if !state.seen.insert(id) {
            return Ok(());
        }
        state.visiting.insert(id);
    }

    let result = (|| -> Result<(), AppError> {
        let merged = merge_inherit(dict, inherited);
        let name = field_name(dict, parent_name);

        if let Some(kids_obj) = dict.get(b"Kids").ok() {
            let Object::Array(kids) = kids_obj else {
                return Err(malformed("A form field /Kids entry is not an array."));
            };
            let kind = classify_field(merged.ft.as_deref(), merged.ff.unwrap_or(0));
            if kind == Some(FormFieldKind::Radio) {
                push_radio(doc, dict, id, &merged, &name, kids, pages, state)?;
                return Ok(());
            }
            for kid in kids {
                if !kid_is_dict(doc, kid) {
                    return Err(malformed("A form field /Kids entry is not a dictionary."));
                }
                walk_node(doc, kid, &merged, &name, depth + 1, None, pages, state)?;
            }
            return Ok(());
        }

        let Some(kind) = classify_field(merged.ft.as_deref(), merged.ff.unwrap_or(0)) else {
            if merged.ft.is_none() {
                return Err(malformed(
                    "A form field is missing a type (/FT) after inheriting from its parent.",
                ));
            }
            return Ok(());
        };

        let widget_ids = if let Some(id) = id {
            vec![id]
        } else {
            Vec::new()
        };
        let (page_index, rect) = widget_geom(doc, dict, &widget_ids, pages);
        let export_values = export_names_from_dict(doc, dict);
        let hidden = widget_hidden(dict);
        state.out.push(WalkedField {
            name,
            kind,
            value_id: id,
            inline_fields_index: if id.is_none() {
                inline_fields_index
            } else {
                None
            },
            widget_ids,
            page_index,
            rect,
            value: merged.v.as_ref().and_then(|v| object_text(doc, v)),
            export_values,
            choices: opt_choices(doc, merged.opt.as_ref()),
            read_only: merged.ff.unwrap_or(0) & FF_READONLY != 0,
            hidden,
            max_len: merged.max_len,
            multiline: merged.ff.unwrap_or(0) & FF_MULTILINE != 0,
            combo_edit: merged.ff.unwrap_or(0) & FF_EDIT != 0,
        });
        Ok(())
    })();

    if let Some(id) = id {
        state.visiting.remove(&id);
    }
    result
}

fn push_radio(
    doc: &Document,
    parent: &Dictionary,
    parent_id: Option<ObjectId>,
    merged: &Inherited,
    name: &str,
    kids: &[Object],
    pages: &BTreeMap<ObjectId, u32>,
    state: &mut WalkState,
) -> Result<(), AppError> {
    let mut widget_ids = Vec::new();
    let mut export_values = Vec::new();
    let mut hidden = widget_hidden(parent);
    let mut first_geom: Option<(Option<u32>, Option<FormRect>)> = None;
    for kid in kids {
        if !kid_is_dict(doc, kid) {
            return Err(malformed("A radio button widget is not a dictionary."));
        }
        let (kd, kid_id) = resolve_dict(doc, kid)?;
        if let Some(id) = kid_id {
            widget_ids.push(id);
        }
        hidden = hidden || widget_hidden(kd);
        for n in export_names_from_dict(doc, kd) {
            if !export_values.iter().any(|e| e == &n) {
                export_values.push(n);
            }
        }
        if first_geom.is_none() {
            first_geom = Some(widget_geom(doc, kd, &widget_ids, pages));
        }
    }
    let (page_index, rect) = first_geom.unwrap_or((None, None));
    state.out.push(WalkedField {
        name: name.to_string(),
        kind: FormFieldKind::Radio,
        value_id: parent_id,
        inline_fields_index: None,
        widget_ids,
        page_index,
        rect,
        value: merged.v.as_ref().and_then(|v| object_text(doc, v)),
        export_values,
        choices: Vec::new(),
        read_only: merged.ff.unwrap_or(0) & FF_READONLY != 0,
        hidden,
        max_len: None,
        multiline: false,
        combo_edit: false,
    });
    Ok(())
}

fn resolve_dict<'a>(
    doc: &'a Document,
    obj: &'a Object,
) -> Result<(&'a Dictionary, Option<ObjectId>), AppError> {
    match obj {
        Object::Dictionary(d) => Ok((d, None)),
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Dictionary(d)) => Ok((d, Some(*id))),
            Ok(_) => Err(malformed("A form field reference does not point at a dictionary.")),
            Err(_) => Err(malformed("A form field reference is missing.")),
        },
        _ => Err(malformed("A form field entry is not a dictionary.")),
    }
}

fn kid_is_dict(doc: &Document, obj: &Object) -> bool {
    match obj {
        Object::Dictionary(_) => true,
        Object::Reference(id) => matches!(doc.get_object(*id), Ok(Object::Dictionary(_))),
        _ => false,
    }
}

fn merge_inherit(dict: &Dictionary, inherited: &Inherited) -> Inherited {
    let mut out = inherited.clone();
    if let Ok(Object::Name(n)) = dict.get(b"FT") {
        out.ft = Some(n.clone());
    }
    if let Some(ff) = dict_u32(dict, b"Ff") {
        out.ff = Some(ff);
    }
    if let Ok(v) = dict.get(b"V") {
        out.v = Some(v.clone());
    }
    if let Ok(da) = dict.get(b"DA") {
        out.da = Some(da.clone());
    }
    if let Some(q) = dict_i64(dict, b"Q") {
        out.q = Some(q);
    }
    if let Ok(opt) = dict.get(b"Opt") {
        out.opt = Some(opt.clone());
    }
    if let Some(m) = dict_u32(dict, b"MaxLen") {
        out.max_len = Some(m);
    }
    out
}

fn field_name(dict: &Dictionary, parent_name: &str) -> String {
    let t = dict_t(dict);
    match t {
        Some(t) if parent_name.is_empty() => t,
        Some(t) => format!("{parent_name}.{t}"),
        None => parent_name.to_string(),
    }
}

fn dict_t(dict: &Dictionary) -> Option<String> {
    match dict.get(b"T").ok()? {
        Object::String(b, _) => Some(decode_pdf_string(b)),
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        _ => None,
    }
}

fn dict_u32(dict: &Dictionary, key: &[u8]) -> Option<u32> {
    match dict.get(key).ok()? {
        Object::Integer(i) if *i >= 0 => Some(*i as u32),
        Object::Real(r) if *r >= 0.0 => Some(*r as u32),
        _ => None,
    }
}

fn dict_i64(dict: &Dictionary, key: &[u8]) -> Option<i64> {
    match dict.get(key).ok()? {
        Object::Integer(i) => Some(*i),
        Object::Real(r) => Some(*r as i64),
        _ => None,
    }
}

fn object_text(doc: &Document, obj: &Object) -> Option<String> {
    match obj {
        Object::String(b, _) => Some(decode_pdf_string(b)),
        Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
        Object::Reference(id) => object_text(doc, doc.get_object(*id).ok()?),
        _ => None,
    }
}

fn widget_hidden(dict: &Dictionary) -> bool {
    let f = dict_i64(dict, b"F").unwrap_or(0);
    f & (F_HIDDEN | F_INVISIBLE | F_NOVIEW) != 0
}

fn widget_geom(
    doc: &Document,
    dict: &Dictionary,
    widget_ids: &[ObjectId],
    pages: &BTreeMap<ObjectId, u32>,
) -> (Option<u32>, Option<FormRect>) {
    let rect = dict_rect(dict);
    let page = page_for_widget(doc, dict, widget_ids, pages);
    (page, rect)
}

fn dict_rect(dict: &Dictionary) -> Option<FormRect> {
    let Object::Array(a) = dict.get(b"Rect").ok()? else {
        return None;
    };
    if a.len() != 4 {
        return None;
    }
    let n = |o: &Object| match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    };
    let llx = n(&a[0])?;
    let lly = n(&a[1])?;
    let urx = n(&a[2])?;
    let ury = n(&a[3])?;
    Some(FormRect {
        x: llx,
        y: lly,
        w: urx - llx,
        h: ury - lly,
    })
}

fn page_for_widget(
    doc: &Document,
    dict: &Dictionary,
    widget_ids: &[ObjectId],
    pages: &BTreeMap<ObjectId, u32>,
) -> Option<u32> {
    if let Ok(Object::Reference(id)) = dict.get(b"P") {
        if let Some(i) = pages.get(id) {
            return Some(*i);
        }
    }
    for (page_id, idx) in pages {
        let Ok(page) = doc.get_dictionary(*page_id) else {
            continue;
        };
        let Ok(Object::Array(annots)) = page.get(b"Annots") else {
            continue;
        };
        for a in annots {
            if let Object::Reference(id) = a {
                if widget_ids.contains(id) {
                    return Some(*idx);
                }
            }
        }
    }
    None
}

fn export_names_from_dict(doc: &Document, dict: &Dictionary) -> Vec<String> {
    let Ok(ap) = dict.get(b"AP") else {
        return Vec::new();
    };
    let Some(ap_dict) = as_dict(doc, ap) else {
        return Vec::new();
    };
    let Ok(n) = ap_dict.get(b"N") else {
        return Vec::new();
    };
    let Some(states) = as_dict(doc, n) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, _) in states.iter() {
        if k != b"Off" {
            out.push(String::from_utf8_lossy(k).into_owned());
        }
    }
    out
}

fn opt_choices(doc: &Document, opt: Option<&Object>) -> Vec<String> {
    let Some(obj) = opt else {
        return Vec::new();
    };
    let arr = match obj {
        Object::Array(a) => a,
        Object::Reference(id) => match doc.get_object(*id) {
            Ok(Object::Array(a)) => a,
            _ => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    arr.iter()
        .filter_map(|o| match o {
            Object::String(b, _) => Some(decode_pdf_string(b)),
            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
            Object::Array(pair) => pair.first().and_then(|x| object_text(doc, x)),
            Object::Reference(id) => object_text(doc, doc.get_object(*id).ok()?),
            _ => None,
        })
        .collect()
}

fn as_dict<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    match obj {
        Object::Dictionary(d) => Some(d),
        Object::Reference(id) => doc.get_dictionary(*id).ok(),
        _ => None,
    }
}

fn apply_form_values_doc(
    doc: &mut Document,
    values: &[FormValue],
    flatten: bool,
) -> Result<(), AppError> {
    if values.is_empty() && !flatten {
        return Ok(());
    }
    let walked = walk_fields(doc)?;
    let font = if needs_text_ap(&walked, values) {
        Some(load_noto()?)
    } else {
        None
    };
    let mut font_id = None;
    if let Some(ref font) = font {
        let used: Vec<char> = values
            .iter()
            .flat_map(|v| v.value.chars())
            .filter(|c| *c != '\n' && *c != '\r')
            .collect();
        font_id = Some(embed_noto(doc, font, &used)?);
    }

    for value in values {
        let Some(field) = match_field(&walked, &value.name) else {
            continue;
        };
        if field.read_only || field.hidden {
            continue;
        }
        write_field_value(doc, field, &value.value, font_id, font.as_ref())?;
    }

    clear_need_appearances(doc)?;
    if flatten {
        flatten_widgets_only(doc)?;
    }
    Ok(())
}

fn needs_text_ap(walked: &[WalkedField], values: &[FormValue]) -> bool {
    values.iter().any(|v| {
        match_field(walked, &v.name)
            .map(|f| {
                !f.read_only
                    && !f.hidden
                    && matches!(
                        f.kind,
                        FormFieldKind::Text | FormFieldKind::Combo | FormFieldKind::List
                    )
            })
            .unwrap_or(false)
    })
}

fn match_field<'a>(walked: &'a [WalkedField], name: &str) -> Option<&'a WalkedField> {
    walked
        .iter()
        .find(|f| f.name == name || f.name.ends_with(&format!(".{name}")))
}

fn write_field_value(
    doc: &mut Document,
    field: &WalkedField,
    value: &str,
    font_id: Option<ObjectId>,
    font: Option<&NotoFont>,
) -> Result<(), AppError> {
    match field.kind {
        FormFieldKind::Checkbox => write_checkbox(doc, field, value),
        FormFieldKind::Radio => write_radio(doc, field, value),
        FormFieldKind::Text | FormFieldKind::Combo | FormFieldKind::List => {
            write_text_like(doc, field, value, font_id, font)
        }
    }
}

fn write_checkbox(doc: &mut Document, field: &WalkedField, value: &str) -> Result<(), AppError> {
    let on = checkbox_on_name(field, value);
    set_field_v(doc, field, Object::Name(on.as_bytes().to_vec()))?;
    for id in btn_widget_targets(field) {
        let mut on_names = widget_export_names(doc, id);
        if on != "Off" && !on_names.iter().any(|e| e == &on) {
            on_names.push(on.clone());
        }
        if on_names.is_empty() {
            on_names.extend(field.export_values.iter().cloned());
        }
        set_btn_widget_ap(doc, id, &on, &on_names, BtnGlyph::Check);
    }
    Ok(())
}

fn checkbox_on_name(field: &WalkedField, value: &str) -> String {
    if value.eq_ignore_ascii_case("Off") || value.is_empty() || value == "false" || value == "0" {
        return "Off".into();
    }
    if field.export_values.iter().any(|e| e == value) {
        return value.to_string();
    }
    field
        .export_values
        .first()
        .cloned()
        .unwrap_or_else(|| value.to_string())
}

fn write_radio(doc: &mut Document, field: &WalkedField, value: &str) -> Result<(), AppError> {
    let selected = if value.eq_ignore_ascii_case("Off") || value.is_empty() {
        "Off".to_string()
    } else {
        value.to_string()
    };
    set_field_v(doc, field, Object::Name(selected.as_bytes().to_vec()))?;
    for id in btn_widget_targets(field) {
        let exports = widget_export_names(doc, id);
        let as_name = if exports.iter().any(|e| e == &selected) {
            selected.clone()
        } else {
            "Off".into()
        };
        let mut on_names = exports;
        if as_name != "Off" && !on_names.iter().any(|e| e == &as_name) {
            on_names.push(as_name.clone());
        }
        set_btn_widget_ap(doc, id, &as_name, &on_names, BtnGlyph::Dot);
    }
    Ok(())
}

/// Targets that hold `/AP` / `/AS` (widget kids, or the field dict itself).
fn btn_widget_targets(field: &WalkedField) -> Vec<ObjectId> {
    let mut targets = field.widget_ids.clone();
    if targets.is_empty() {
        if let Some(id) = field.value_id {
            targets.push(id);
        }
    }
    targets
}

fn widget_export_names(doc: &Document, id: ObjectId) -> Vec<String> {
    let Ok(Object::Dictionary(d)) = doc.get_object(id) else {
        return Vec::new();
    };
    export_names_from_dict(doc, d)
}

/// Drawn Form XObject for `/Off` and each on-export name. Name-only `/AP /N`
/// (Preview/Chrome invisible) is replaced; `/V`/`/AS` stay the export name.
fn set_btn_widget_ap(
    doc: &mut Document,
    widget_id: ObjectId,
    as_name: &str,
    on_names: &[String],
    on_glyph: BtnGlyph,
) {
    let (w, h) = widget_ap_size(doc, widget_id);
    let off_id = build_btn_form_xobject(doc, w, h, BtnGlyph::Off);
    let mut n = Dictionary::new();
    n.set("Off", Object::Reference(off_id));
    for name in on_names {
        if name == "Off" {
            continue;
        }
        let on_id = build_btn_form_xobject(doc, w, h, on_glyph);
        n.set(name.as_bytes().to_vec(), Object::Reference(on_id));
    }
    if let Ok(Object::Dictionary(d)) = doc.get_object_mut(widget_id) {
        let mut ap = Dictionary::new();
        ap.set("N", Object::Dictionary(n));
        d.set("AP", Object::Dictionary(ap));
        d.set("AS", Object::Name(as_name.as_bytes().to_vec()));
    }
}

fn widget_ap_size(doc: &Document, id: ObjectId) -> (f64, f64) {
    if let Ok(Object::Dictionary(d)) = doc.get_object(id) {
        if let Some(r) = dict_rect(d) {
            return (r.w.abs().max(1.0), r.h.abs().max(1.0));
        }
    }
    (12.0, 12.0)
}

#[derive(Clone, Copy)]
enum BtnGlyph {
    Off,
    Check,
    Dot,
}

fn build_btn_form_xobject(doc: &mut Document, w: f64, h: f64, glyph: BtnGlyph) -> ObjectId {
    let content = btn_ap_ops(w, h, glyph);
    let mut d = Dictionary::new();
    d.set("Type", "XObject");
    d.set("Subtype", Object::Name(b"Form".to_vec()));
    d.set("FormType", Object::Integer(1));
    d.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(w as f32),
            Object::Real(h as f32),
        ]),
    );
    d.set("Resources", Object::Dictionary(Dictionary::new()));
    d.set("Length", Object::Integer(content.len() as i64));
    doc.add_object(Object::Stream(Stream::new(d, content)))
}

fn btn_ap_ops(w: f64, h: f64, glyph: BtnGlyph) -> Vec<u8> {
    let w = w.max(1.0);
    let h = h.max(1.0);
    let pad = (w.min(h) * 0.08).max(0.4);
    match glyph {
        BtnGlyph::Off => format!("q\n0 0 {w:.2} {h:.2} re\nW n\nQ\n").into_bytes(),
        BtnGlyph::Check => {
            let lw = (w.min(h) * 0.12).max(0.8);
            let x1 = pad + w * 0.12;
            let y1 = h * 0.48;
            let x2 = w * 0.40;
            let y2 = pad + h * 0.12;
            let x3 = w - pad - w * 0.08;
            let y3 = h - pad - h * 0.12;
            format!(
                "q\n0 0 {w:.2} {h:.2} re\nW n\n0 g\n{lw:.2} w\n1 j 1 J\n{x1:.2} {y1:.2} m\n{x2:.2} {y2:.2} l\n{x3:.2} {y3:.2} l\nS\nQ\n"
            )
            .into_bytes()
        }
        BtnGlyph::Dot => {
            let cx = w * 0.5;
            let cy = h * 0.5;
            let r = (w.min(h) * 0.22).max(0.6);
            let k = 0.552_284_75 * r;
            format!(
                "q\n0 0 {w:.2} {h:.2} re\nW n\n0 g\n{cx:.2} {y0:.2} m\n{x1:.2} {y0:.2} {x2:.2} {y1:.2} {x2:.2} {cy:.2} c\n{x2:.2} {y3:.2} {x1:.2} {y4:.2} {cx:.2} {y4:.2} c\n{x3:.2} {y4:.2} {x0:.2} {y3:.2} {x0:.2} {cy:.2} c\n{x0:.2} {y1:.2} {x3:.2} {y0:.2} {cx:.2} {y0:.2} c\nf\nQ\n",
                x0 = cx - r,
                x1 = cx + k,
                x2 = cx + r,
                x3 = cx - k,
                y0 = cy - r,
                y1 = cy - k,
                y3 = cy + k,
                y4 = cy + r,
            )
            .into_bytes()
        }
    }
}

fn write_text_like(
    doc: &mut Document,
    field: &WalkedField,
    value: &str,
    font_id: Option<ObjectId>,
    font: Option<&NotoFont>,
) -> Result<(), AppError> {
    set_field_v(
        doc,
        field,
        Object::String(encode_pdf_string(value), lopdf::StringFormat::Literal),
    )?;
    let (Some(font_id), Some(font)) = (font_id, font) else {
        return Ok(());
    };
    let rect = field.rect.clone().unwrap_or(FormRect {
        x: 0.0,
        y: 0.0,
        w: 120.0,
        h: 20.0,
    });
    let ap_id = build_text_ap(doc, font, font_id, value, rect.w, rect.h, field.multiline)?;
    let mut targets = field.widget_ids.clone();
    if targets.is_empty() {
        if let Some(id) = field.value_id {
            targets.push(id);
        }
    }
    for id in targets {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(id) {
            let mut n = Dictionary::new();
            n.set("N", Object::Reference(ap_id));
            d.set("AP", Object::Dictionary(n));
        }
    }
    Ok(())
}

fn set_field_v(doc: &mut Document, field: &WalkedField, v: Object) -> Result<(), AppError> {
    if let Some(id) = field.value_id {
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(id) {
            d.set("V", v);
            return Ok(());
        }
    }
    if let Some(idx) = field.inline_fields_index {
        set_inline_field_v(doc, idx, v)?;
    }
    Ok(())
}

fn set_inline_field_v(doc: &mut Document, index: usize, v: Object) -> Result<(), AppError> {
    let root = doc
        .trailer
        .get(b"Root")
        .ok()
        .and_then(|o| o.as_reference().ok())
        .ok_or_else(|| malformed("The PDF catalog is missing."))?;
    let acro_ref = {
        let cat = doc
            .get_dictionary(root)
            .map_err(|_| malformed("The PDF catalog is missing."))?;
        match cat.get(b"AcroForm").ok() {
            Some(Object::Reference(id)) => Some(*id),
            _ => None,
        }
    };
    if let Some(id) = acro_ref {
        if let Ok(Object::Dictionary(acro)) = doc.get_object_mut(id) {
            if let Ok(Object::Array(fields)) = acro.get_mut(b"Fields") {
                if let Some(Object::Dictionary(d)) = fields.get_mut(index) {
                    d.set("V", v);
                }
            }
        }
        return Ok(());
    }
    Ok(())
}

fn clear_need_appearances(doc: &mut Document) -> Result<(), AppError> {
    let root = match doc.trailer.get(b"Root").ok().and_then(|o| o.as_reference().ok()) {
        Some(id) => id,
        None => return Ok(()),
    };
    let acro_id = match doc.get_dictionary(root) {
        Ok(cat) => match cat.get(b"AcroForm").ok() {
            Some(Object::Reference(id)) => Some(*id),
            _ => None,
        },
        Err(_) => None,
    };
    if let Some(id) = acro_id {
        if let Ok(Object::Dictionary(acro)) = doc.get_object_mut(id) {
            acro.set("NeedAppearances", false);
        }
    }
    Ok(())
}

fn flatten_widgets_only(doc: &mut Document) -> Result<(), AppError> {
    let page_map = doc.get_pages();
    let page_ids: Vec<ObjectId> = page_map.values().copied().collect();
    for page_id in page_ids {
        flatten_page_widgets(doc, page_id)?;
    }
    Ok(())
}

fn flatten_page_widgets(doc: &mut Document, page_id: ObjectId) -> Result<(), AppError> {
    let annots = {
        let Ok(page) = doc.get_dictionary(page_id) else {
            return Ok(());
        };
        match page.get(b"Annots") {
            Ok(Object::Array(a)) => a.clone(),
            _ => return Ok(()),
        }
    };

    let mut keep = Vec::new();
    let mut paints: Vec<(FormRect, ObjectId)> = Vec::new();
    for annot in &annots {
        let (dict, _) = match resolve_dict(doc, annot) {
            Ok(v) => v,
            Err(_) => {
                keep.push(annot.clone());
                continue;
            }
        };
        let subtype = match dict.get(b"Subtype") {
            Ok(Object::Name(n)) => n.as_slice(),
            _ => {
                keep.push(annot.clone());
                continue;
            }
        };
        if subtype != b"Widget" {
            keep.push(annot.clone());
            continue;
        }
        if let (Some(rect), Some(ap_id)) = (dict_rect(dict), appearance_stream_id(doc, dict)) {
            paints.push((rect, ap_id));
        }
    }

    if paints.is_empty() && keep.len() == annots.len() {
        return Ok(());
    }

    let mut xobjects = Dictionary::new();
    let mut ops = String::new();
    for (i, (rect, ap_id)) in paints.iter().enumerate() {
        let name = format!("Ff{i}");
        xobjects.set(name.as_bytes().to_vec(), Object::Reference(*ap_id));
        // Form BBox is [0 0 w h]; place at the widget's lower-left.
        ops.push_str(&format!(
            "q\n1 0 0 1 {:.4} {:.4} cm\n/{name} Do\nQ\n",
            rect.x, rect.y
        ));
    }

    if !paints.is_empty() {
        attach_xobjects_and_content(doc, page_id, xobjects, ops.into_bytes())?;
    }

    if let Ok(Object::Dictionary(page)) = doc.get_object_mut(page_id) {
        if keep.is_empty() {
            page.remove(b"Annots");
        } else {
            page.set("Annots", Object::Array(keep));
        }
    }
    Ok(())
}

fn appearance_stream_id(doc: &Document, dict: &Dictionary) -> Option<ObjectId> {
    let ap = dict.get(b"AP").ok()?;
    let ap_dict = as_dict(doc, ap)?;
    let n = ap_dict.get(b"N").ok()?;
    match n {
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Stream(_) => Some(*id),
            Object::Dictionary(states) => pick_state_stream(doc, dict, states),
            _ => None,
        },
        Object::Dictionary(states) => pick_state_stream(doc, dict, states),
        _ => None,
    }
}

fn pick_state_stream(_doc: &Document, widget: &Dictionary, states: &Dictionary) -> Option<ObjectId> {
    let as_name = match widget.get(b"AS") {
        Ok(Object::Name(n)) => n.clone(),
        _ => b"Off".to_vec(),
    };
    let pick = states.get(&as_name).ok().or_else(|| {
        states
            .iter()
            .find(|(k, _)| *k != b"Off")
            .map(|(_, v)| v)
    })?;
    pick.as_reference().ok()
}

fn attach_xobjects_and_content(
    doc: &mut Document,
    page_id: ObjectId,
    xobjects: Dictionary,
    ops: Vec<u8>,
) -> Result<(), AppError> {
    let xo_id = doc.add_object(Object::Dictionary(xobjects));
    let mut stream_dict = Dictionary::new();
    stream_dict.set("Length", Object::Integer(ops.len() as i64));
    let content_id = doc.add_object(Object::Stream(Stream::new(stream_dict, ops)));

    let resources_id = {
        let page = doc
            .get_dictionary(page_id)
            .map_err(|e| AppError::engine_failed(format!("page: {e}")))?;
        match page.get(b"Resources").ok() {
            Some(Object::Reference(id)) => Some(*id),
            _ => None,
        }
    };

    if let Some(rid) = resources_id {
        if let Ok(Object::Dictionary(res)) = doc.get_object_mut(rid) {
            res.set("XObject", Object::Reference(xo_id));
        }
    } else if let Ok(Object::Dictionary(page)) = doc.get_object_mut(page_id) {
        let mut res = Dictionary::new();
        if let Ok(existing) = page.get(b"Resources") {
            if let Object::Dictionary(d) = existing {
                res = d.clone();
            }
        }
        res.set("XObject", Object::Reference(xo_id));
        page.set("Resources", Object::Dictionary(res));
    }

    let existing = {
        let page = doc
            .get_dictionary(page_id)
            .map_err(|e| AppError::engine_failed(format!("page: {e}")))?;
        page.get(b"Contents").ok().cloned()
    };
    if let Ok(Object::Dictionary(page)) = doc.get_object_mut(page_id) {
        let mut arr = match existing {
            Some(Object::Array(a)) => a,
            Some(Object::Reference(id)) => vec![Object::Reference(id)],
            Some(other) => vec![other],
            None => Vec::new(),
        };
        arr.push(Object::Reference(content_id));
        page.set("Contents", Object::Array(arr));
    }
    Ok(())
}

struct NotoFont {
    data: Vec<u8>,
    units_per_em: f64,
    bbox: [i16; 4],
    ascent: i16,
    descent: i16,
}

impl NotoFont {
    fn parse(data: Vec<u8>) -> Result<Self, AppError> {
        let face = Face::parse(&data, 0).map_err(|_| {
            AppError::new(
                "BAD_FONT",
                "Editor font unreadable",
                "The bundled font is damaged.",
            )
        })?;
        let bb = face.global_bounding_box();
        Ok(Self {
            units_per_em: face.units_per_em() as f64,
            bbox: [bb.x_min, bb.y_min, bb.x_max, bb.y_max],
            ascent: face.ascender(),
            descent: face.descender(),
            data,
        })
    }
    fn face(&self) -> Face<'_> {
        Face::parse(&self.data, 0).expect("font already parsed")
    }
    fn gid(&self, ch: char) -> u16 {
        self.face().glyph_index(ch).map(|g| g.0).unwrap_or(0)
    }
    fn width(&self, gid: u16) -> f64 {
        let adv = self.face().glyph_hor_advance(GlyphId(gid)).unwrap_or(0) as f64;
        adv / self.units_per_em * 1000.0
    }
    fn scale(&self) -> f64 {
        1000.0 / self.units_per_em
    }
}

fn load_noto() -> Result<NotoFont, AppError> {
    let path = noto_path()?;
    let data = std::fs::read(&path).map_err(|e| AppError::io("Could not read the editor font.", e))?;
    NotoFont::parse(data)
}

fn noto_path() -> Result<PathBuf, AppError> {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("fonts")
        .join("NotoSans-Regular.ttf");
    if dev.exists() {
        return Ok(dev);
    }
    Err(AppError::new(
        "NO_FONT",
        "Editor font missing",
        "The bundled Noto Sans font could not be found.",
    )
    .with_suggestion("Reinstall OffPDF."))
}

fn embed_noto(doc: &mut Document, font: &NotoFont, used: &[char]) -> Result<ObjectId, AppError> {
    let mut pairs: Vec<(u16, char)> = used.iter().map(|c| (font.gid(*c), *c)).collect();
    pairs.sort_by_key(|(g, _)| *g);
    pairs.dedup_by_key(|(g, _)| *g);

    let mut fontfile_dict = Dictionary::new();
    fontfile_dict.set("Length", Object::Integer(font.data.len() as i64));
    fontfile_dict.set("Length1", Object::Integer(font.data.len() as i64));
    let fontfile = doc.add_object(Object::Stream(Stream::new(fontfile_dict, font.data.clone())));

    let sc = font.scale();
    let bb: [i32; 4] = font.bbox.map(|v| (v as f64 * sc).round() as i32);
    let ascent = (font.ascent as f64 * sc).round() as i32;
    let descent = (font.descent as f64 * sc).round() as i32;
    let mut desc = Dictionary::new();
    desc.set("Type", "FontDescriptor");
    desc.set("FontName", Object::Name(b"NotoSans".to_vec()));
    desc.set("Flags", 32);
    desc.set(
        "FontBBox",
        Object::Array(bb.into_iter().map(|n| Object::Integer(n as i64)).collect()),
    );
    desc.set("ItalicAngle", 0);
    desc.set("Ascent", ascent);
    desc.set("Descent", descent);
    desc.set("CapHeight", ascent);
    desc.set("StemV", 80);
    desc.set("FontFile2", Object::Reference(fontfile));
    let desc_id = doc.add_object(Object::Dictionary(desc));

    let mut w_arr: Vec<Object> = Vec::new();
    for (gid, _) in &pairs {
        if *gid != 0 {
            w_arr.push(Object::Integer(*gid as i64));
            w_arr.push(Object::Array(vec![Object::Integer(font.width(*gid) as i64)]));
        }
    }
    let mut cid_sys = Dictionary::new();
    cid_sys.set("Registry", Object::string_literal("Adobe"));
    cid_sys.set("Ordering", Object::string_literal("Identity"));
    cid_sys.set("Supplement", 0);
    let mut cid = Dictionary::new();
    cid.set("Type", "Font");
    cid.set("Subtype", Object::Name(b"CIDFontType2".to_vec()));
    cid.set("BaseFont", Object::Name(b"NotoSans".to_vec()));
    cid.set("CIDSystemInfo", Object::Dictionary(cid_sys));
    cid.set("FontDescriptor", Object::Reference(desc_id));
    cid.set("DW", 600);
    cid.set("W", Object::Array(w_arr));
    cid.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
    let cid_id = doc.add_object(Object::Dictionary(cid));

    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let pairs_nz: Vec<(u16, char)> = pairs.iter().copied().filter(|(g, _)| *g != 0).collect();
    for chunk in pairs_nz.chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (gid, ch) in chunk {
            let cp = *ch as u32;
            if cp <= 0xFFFF {
                cmap.push_str(&format!("<{gid:04X}> <{cp:04X}>\n"));
            } else {
                let u = cp - 0x10000;
                let hi = 0xD800 + (u >> 10);
                let lo = 0xDC00 + (u & 0x3FF);
                cmap.push_str(&format!("<{gid:04X}> <{hi:04X}{lo:04X}>\n"));
            }
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    let touni = doc.add_object(Object::Stream(Stream::new(
        Dictionary::new(),
        cmap.into_bytes(),
    )));

    let mut type0 = Dictionary::new();
    type0.set("Type", "Font");
    type0.set("Subtype", Object::Name(b"Type0".to_vec()));
    type0.set("BaseFont", Object::Name(b"NotoSans".to_vec()));
    type0.set("Encoding", Object::Name(b"Identity-H".to_vec()));
    type0.set(
        "DescendantFonts",
        Object::Array(vec![Object::Reference(cid_id)]),
    );
    type0.set("ToUnicode", Object::Reference(touni));
    Ok(doc.add_object(Object::Dictionary(type0)))
}

fn build_text_ap(
    doc: &mut Document,
    font: &NotoFont,
    font_id: ObjectId,
    text: &str,
    w: f64,
    h: f64,
    multiline: bool,
) -> Result<ObjectId, AppError> {
    let size = if h > 6.0 { (h - 4.0).clamp(8.0, 12.0) } else { 8.0 };
    let lines = if multiline {
        wrap_ap_text(font, text, size, (w - 4.0).max(8.0))
    } else {
        vec![text.replace('\n', " ")]
    };
    let mut content = String::from("/Tx BMC\nq\n");
    content.push_str(&format!("0 0 {w:.2} {h:.2} re W n\n"));
    content.push_str("BT\n/F1 ");
    content.push_str(&format!("{size:.2} Tf\n"));
    let line_h = size * 1.15;
    let mut y = (h - size - 2.0).max(2.0);
    for line in &lines {
        let (hex, _) = line_hex(font, line);
        content.push_str(&format!("2 {y:.2} Td\n<{hex}> Tj\n"));
        content.push_str(&format!("-2 {:.2} Td\n", -line_h));
        y -= line_h;
        if y < 0.0 && multiline {
            break;
        }
    }
    content.push_str("ET\nQ\nEMC\n");

    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));
    let mut d = Dictionary::new();
    d.set("Type", "XObject");
    d.set("Subtype", Object::Name(b"Form".to_vec()));
    d.set(
        "BBox",
        Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(w as f32),
            Object::Real(h as f32),
        ]),
    );
    d.set("Resources", Object::Dictionary(resources));
    d.set("Length", Object::Integer(content.len() as i64));
    Ok(doc.add_object(Object::Stream(Stream::new(d, content.into_bytes()))))
}

fn wrap_ap_text(font: &NotoFont, text: &str, font_size: f64, max_w: f64) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut cur = String::new();
        let mut cur_w = 0.0;
        for ch in para.chars() {
            let cw = font.width(font.gid(ch)) * font_size / 1000.0;
            if !cur.is_empty() && cur_w + cw > max_w && max_w > 8.0 {
                lines.push(std::mem::take(&mut cur));
                cur_w = 0.0;
            }
            cur.push(ch);
            cur_w += cw;
        }
        lines.push(cur);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn line_hex(font: &NotoFont, text: &str) -> (String, f64) {
    let mut hex = String::new();
    let mut w = 0.0;
    for ch in text.chars() {
        let gid = font.gid(ch);
        hex.push_str(&format!("{gid:04X}"));
        w += font.width(gid);
    }
    (hex, w)
}

fn snapshot_for_form_dest(
    source: &str,
    dest: &Path,
    flatten: bool,
) -> Result<OutputSnapshot, AppError> {
    let src = load_doc(source)?;
    let dest_doc = Document::load(dest)
        .map_err(|e| AppError::engine_failed(format!("Could not reopen the filled PDF: {e}")))?;
    let mut pages = Vec::new();
    let src_pages = src.get_pages();
    let mut nums: Vec<u32> = src_pages.keys().copied().collect();
    nums.sort_unstable();
    for n in nums {
        let id = *src_pages.get(&n).expect("page");
        let content = src
            .get_page_content(id)
            .map_err(|e| AppError::engine_failed(format!("Could not read page content: {e}")))?;
        pages.push(PageSnapshot {
            media_box: crop::media_box(&src, id),
            crop_box: crop::crop_box(&src, id),
            trim_box: crop::page_trim_box(&src, id),
            rotate: crop::page_rotation(&src, id),
            user_unit: crop::page_user_unit(&src, id),
            content_digest: content_digest(&content),
        });
    }
    let mut catalog = catalog_flags_from_doc(&src);
    if flatten {
        catalog.acro_form = catalog_flags_from_doc(&dest_doc).acro_form;
    }
    Ok(OutputSnapshot { pages, catalog })
}

pub(crate) fn qpdf_rewrite(input: &Path, output: &Path) -> Result<(), AppError> {
    let exe = qpdf::resolve_qpdf_standalone();
    let status = std::process::Command::new(&exe)
        .arg(input.as_os_str())
        .arg(output.as_os_str())
        .status()
        .map_err(|e| AppError::io("qpdf failed to start", e))?;
    if !status.success() {
        return Err(AppError::engine_failed(format!(
            "qpdf rewrite failed with status {status}"
        )));
    }
    Ok(())
}

fn run_qpdf_check(exe: &Path, args: &[String]) -> Result<(i32, String), AppError> {
    let output = std::process::Command::new(exe)
        .args(args)
        .output()
        .map_err(|e| AppError::io("qpdf --check failed to start", e))?;
    Ok((
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AppError;
    use crate::pdf_engine::metadata::decode_pdf_string;
    use crate::pdf_engine::validate_output::{
        catalog_flags_from_doc, content_digest, validate_staged_pdf, CatalogFlags, OutputSnapshot,
        PageSnapshot,
    };
    use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
    use std::path::{Path, PathBuf};

    // PDF 1-based flag bits (spec Table 221 / 226 / 231).
    const FF_READONLY: u32 = 1 << 0;
    const FF_MULTILINE: u32 = 1 << 12;
    const FF_RADIO: u32 = 1 << 15;
    const FF_PUSHBUTTON: u32 = 1 << 16;
    const FF_COMBO: u32 = 1 << 17;
    const FF_EDIT: u32 = 1 << 18;
    const F_INVISIBLE: i64 = 1 << 0;
    const F_HIDDEN: i64 = 1 << 1;
    const F_NOVIEW: i64 = 1 << 5;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "offpdf-forms-{}-{}-{}",
                name,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn pdf(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn box_obj(b: [i64; 4]) -> Object {
        Object::Array(b.into_iter().map(Object::Integer).collect())
    }

    fn pdf_name(s: &str) -> Object {
        Object::Name(s.as_bytes().to_vec())
    }

    fn kinds_of(fields: &[FormField]) -> Vec<FormFieldKind> {
        fields.iter().map(|f| f.kind).collect()
    }

    fn names_of(fields: &[FormField]) -> Vec<String> {
        fields.iter().map(|f| f.name.clone()).collect()
    }

    fn assert_actionable(err: &AppError, code: &str, ctx: &str) {
        assert_eq!(err.code, code, "{ctx}: code");
        assert!(
            !err.title.trim().is_empty(),
            "{ctx}: AppError must have a title"
        );
        assert!(
            !err.message.trim().is_empty(),
            "{ctx}: AppError must have a message"
        );
        assert!(
            err.suggestion
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false),
            "{ctx}: AppError must have a suggestion"
        );
    }

    fn letter_snapshot() -> OutputSnapshot {
        OutputSnapshot {
            pages: vec![PageSnapshot {
                media_box: [0.0, 0.0, 612.0, 792.0],
                crop_box: None,
                trim_box: None,
                rotate: 0,
                user_unit: 1.0,
                content_digest: content_digest(b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET"),
            }],
            catalog: CatalogFlags {
                outlines: false,
                info: false,
                acro_form: true,
                annots: true,
            },
        }
    }

    struct FormDoc {
        doc: Document,
        page_id: ObjectId,
        pages_id: ObjectId,
        fields: Vec<Object>,
        annots: Vec<Object>,
        outlines: Option<ObjectId>,
        info: Option<ObjectId>,
        leftover_annots: Vec<Object>,
        need_appearances: bool,
        xfa: Option<Object>,
        needs_rendering: bool,
        rotate: i64,
        crop: Option<[i64; 4]>,
    }

    impl FormDoc {
        fn new() -> Self {
            let mut doc = Document::with_version("1.5");
            let pages_id = doc.new_object_id();
            let content_id = doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                b"BT /F1 12 Tf 72 720 Td (Hello) Tj ET".to_vec(),
            )));
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", pages_id);
            page.set("MediaBox", box_obj([0, 0, 612, 792]));
            page.set("Contents", content_id);
            let page_id = doc.add_object(Object::Dictionary(page));
            Self {
                doc,
                page_id,
                pages_id,
                fields: Vec::new(),
                annots: Vec::new(),
                outlines: None,
                info: None,
                leftover_annots: Vec::new(),
                need_appearances: false,
                xfa: None,
                needs_rendering: false,
                rotate: 0,
                crop: None,
            }
        }

        fn with_catalog_extras(&mut self) {
            let mut item = Dictionary::new();
            item.set("Title", Object::string_literal("Chapter 1"));
            item.set(
                "Dest",
                vec![self.page_id.into(), Object::Name(b"Fit".to_vec())],
            );
            let item_id = self.doc.add_object(Object::Dictionary(item));
            let mut outlines = Dictionary::new();
            outlines.set("Type", "Outlines");
            outlines.set("First", item_id);
            outlines.set("Last", item_id);
            outlines.set("Count", 1);
            let outlines_id = self.doc.add_object(Object::Dictionary(outlines));
            if let Ok(Object::Dictionary(d)) = self.doc.get_object_mut(item_id) {
                d.set("Parent", outlines_id);
            }
            self.outlines = Some(outlines_id);

            let mut info = Dictionary::new();
            info.set("Title", Object::string_literal("Fixture Doc"));
            info.set("Author", Object::string_literal("OffPDF"));
            self.info = Some(self.doc.add_object(Object::Dictionary(info)));
        }

        fn add_text_ap(&mut self, label: &str) -> Object {
            let stream = self.doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                format!("BT /F1 10 Tf 2 2 Td ({label}) Tj ET").into_bytes(),
            )));
            let mut n = Dictionary::new();
            n.set("N", stream);
            Object::Dictionary(n)
        }

        fn add_btn_ap(&mut self, on_name: &str) -> Object {
            let on = self.doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                format!("BT ({on_name}) Tj ET").into_bytes(),
            )));
            let off = self.doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                b"BT (Off) Tj ET".to_vec(),
            )));
            let mut n = Dictionary::new();
            n.set(on_name, on);
            n.set("Off", off);
            let mut ap = Dictionary::new();
            ap.set("N", Object::Dictionary(n));
            Object::Dictionary(ap)
        }

        fn push_merged_text(
            &mut self,
            name: &str,
            value: &str,
            rect: [i64; 4],
            ap_label: &str,
        ) -> ObjectId {
            let ap = self.add_text_ap(ap_label);
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Tx"));
            d.set("T", Object::string_literal(name));
            d.set("V", Object::string_literal(value));
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj(rect));
            d.set("AP", ap);
            d.set("DA", Object::string_literal("/Helv 10 Tf 0 g"));
            d.set("P", self.page_id);
            let id = self.doc.add_object(Object::Dictionary(d));
            self.fields.push(id.into());
            self.annots.push(id.into());
            id
        }

        fn push_checkbox(
            &mut self,
            name: &str,
            on_name: &str,
            on: bool,
            rect: [i64; 4],
        ) -> ObjectId {
            let ap = self.add_btn_ap(on_name);
            let state = if on { on_name } else { "Off" };
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Btn"));
            d.set("T", Object::string_literal(name));
            d.set("V", pdf_name(state));
            d.set("AS", pdf_name(state));
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj(rect));
            d.set("AP", ap);
            d.set("P", self.page_id);
            let id = self.doc.add_object(Object::Dictionary(d));
            self.fields.push(id.into());
            self.annots.push(id.into());
            id
        }

        /// QA-form-fill style: `/AP << /N << /Yes /Yes /Off /Off >> >>` (names, not streams).
        fn push_checkbox_name_only_ap(
            &mut self,
            name: &str,
            on_name: &str,
            on: bool,
            rect: [i64; 4],
        ) -> ObjectId {
            let mut n = Dictionary::new();
            n.set(on_name, pdf_name(on_name));
            n.set("Off", pdf_name("Off"));
            let mut ap = Dictionary::new();
            ap.set("N", Object::Dictionary(n));
            let state = if on { on_name } else { "Off" };
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Btn"));
            d.set("T", Object::string_literal(name));
            d.set("V", pdf_name(state));
            d.set("AS", pdf_name(state));
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj(rect));
            d.set("AP", Object::Dictionary(ap));
            d.set("P", self.page_id);
            let id = self.doc.add_object(Object::Dictionary(d));
            self.fields.push(id.into());
            self.annots.push(id.into());
            id
        }

        fn push_radio_group(
            &mut self,
            name: &str,
            options: &[&str],
            selected: &str,
            origin: [i64; 4],
        ) -> ObjectId {
            let mut kids = Vec::new();
            for (i, opt) in options.iter().enumerate() {
                let ap = self.add_btn_ap(opt);
                let state = if *opt == selected { *opt } else { "Off" };
                let mut w = Dictionary::new();
                w.set("Type", "Annot");
                w.set("Subtype", "Widget");
                w.set(
                    "Rect",
                    box_obj([
                        origin[0],
                        origin[1] - (i as i64) * 24,
                        origin[2],
                        origin[3] - (i as i64) * 24,
                    ]),
                );
                w.set("AP", ap);
                w.set("AS", pdf_name(state));
                w.set("Parent", Object::Reference((0, 0)));
                w.set("P", self.page_id);
                let id = self.doc.add_object(Object::Dictionary(w));
                kids.push(id);
                self.annots.push(id.into());
            }
            let mut parent = Dictionary::new();
            parent.set("FT", pdf_name("Btn"));
            parent.set("Ff", Object::Integer(FF_RADIO as i64));
            parent.set("T", Object::string_literal(name));
            parent.set("V", pdf_name(selected));
            parent.set(
                "Kids",
                kids.iter().copied().map(Object::from).collect::<Vec<_>>(),
            );
            let parent_id = self.doc.add_object(Object::Dictionary(parent));
            for kid in &kids {
                if let Ok(Object::Dictionary(d)) = self.doc.get_object_mut(*kid) {
                    d.set("Parent", parent_id);
                }
            }
            self.fields.push(parent_id.into());
            parent_id
        }

        /// QA-form-fill style radio: each kid `/AP /N /<export>` is a Name, not a stream.
        fn push_radio_name_only_ap(
            &mut self,
            name: &str,
            options: &[&str],
            selected: &str,
            origin: [i64; 4],
        ) -> ObjectId {
            let mut kids = Vec::new();
            for (i, opt) in options.iter().enumerate() {
                let mut n = Dictionary::new();
                n.set(*opt, pdf_name(opt));
                n.set("Off", pdf_name("Off"));
                let mut ap = Dictionary::new();
                ap.set("N", Object::Dictionary(n));
                let state = if *opt == selected { *opt } else { "Off" };
                let mut w = Dictionary::new();
                w.set("Type", "Annot");
                w.set("Subtype", "Widget");
                w.set(
                    "Rect",
                    box_obj([
                        origin[0],
                        origin[1] - (i as i64) * 24,
                        origin[2],
                        origin[3] - (i as i64) * 24,
                    ]),
                );
                w.set("AP", Object::Dictionary(ap));
                w.set("AS", pdf_name(state));
                w.set("Parent", Object::Reference((0, 0)));
                w.set("P", self.page_id);
                let id = self.doc.add_object(Object::Dictionary(w));
                kids.push(id);
                self.annots.push(id.into());
            }
            let mut parent = Dictionary::new();
            parent.set("FT", pdf_name("Btn"));
            parent.set("Ff", Object::Integer(FF_RADIO as i64));
            parent.set("T", Object::string_literal(name));
            parent.set("V", pdf_name(selected));
            parent.set(
                "Kids",
                kids.iter().copied().map(Object::from).collect::<Vec<_>>(),
            );
            let parent_id = self.doc.add_object(Object::Dictionary(parent));
            for kid in &kids {
                if let Ok(Object::Dictionary(d)) = self.doc.get_object_mut(*kid) {
                    d.set("Parent", parent_id);
                }
            }
            self.fields.push(parent_id.into());
            parent_id
        }

        fn push_choice(
            &mut self,
            name: &str,
            value: &str,
            opts: &[&str],
            combo: bool,
            rect: [i64; 4],
        ) -> ObjectId {
            let ap = self.add_text_ap(value);
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Ch"));
            if combo {
                d.set("Ff", Object::Integer(FF_COMBO as i64));
            }
            d.set("T", Object::string_literal(name));
            d.set("V", Object::string_literal(value));
            d.set(
                "Opt",
                opts.iter()
                    .map(|s| Object::string_literal(*s))
                    .collect::<Vec<_>>(),
            );
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj(rect));
            d.set("AP", ap);
            d.set("P", self.page_id);
            let id = self.doc.add_object(Object::Dictionary(d));
            self.fields.push(id.into());
            self.annots.push(id.into());
            id
        }

        fn push_pushbutton(&mut self, name: &str, rect: [i64; 4]) {
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Btn"));
            d.set("Ff", Object::Integer(FF_PUSHBUTTON as i64));
            d.set("T", Object::string_literal(name));
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj(rect));
            d.set("P", self.page_id);
            let id = self.doc.add_object(Object::Dictionary(d));
            self.fields.push(id.into());
            self.annots.push(id.into());
        }

        fn push_sig(&mut self, name: &str, rect: [i64; 4]) {
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Sig"));
            d.set("T", Object::string_literal(name));
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj(rect));
            d.set("P", self.page_id);
            let id = self.doc.add_object(Object::Dictionary(d));
            self.fields.push(id.into());
            self.annots.push(id.into());
        }

        fn push_leftover_text(&mut self) {
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", "Text");
            annot.set("Rect", box_obj([72, 700, 120, 740]));
            annot.set("Contents", Object::string_literal("note"));
            let id = self.doc.add_object(Object::Dictionary(annot));
            self.leftover_annots.push(id.into());
        }

        fn push_leftover_link(&mut self) {
            let mut action = Dictionary::new();
            action.set("S", pdf_name("URI"));
            action.set("URI", Object::string_literal("https://keep.example/"));
            let mut annot = Dictionary::new();
            annot.set("Type", "Annot");
            annot.set("Subtype", "Link");
            annot.set("Rect", box_obj([200, 700, 280, 720]));
            annot.set("A", Object::Dictionary(action));
            let id = self.doc.add_object(Object::Dictionary(annot));
            self.leftover_annots.push(id.into());
        }

        fn save(mut self, path: &Path) {
            if let Ok(Object::Dictionary(page)) = self.doc.get_object_mut(self.page_id) {
                if self.rotate != 0 {
                    page.set("Rotate", Object::Integer(self.rotate));
                }
                if let Some(crop) = self.crop {
                    page.set("CropBox", box_obj(crop));
                }
                let mut annots = self.annots.clone();
                annots.extend(self.leftover_annots.iter().cloned());
                if !annots.is_empty() {
                    page.set("Annots", annots);
                }
            }

            let mut pages = Dictionary::new();
            pages.set("Type", "Pages");
            pages.set("Kids", vec![self.page_id.into()]);
            pages.set("Count", 1);
            self.doc
                .objects
                .insert(self.pages_id, Object::Dictionary(pages));

            let mut acro = Dictionary::new();
            acro.set("Fields", self.fields);
            if self.need_appearances {
                acro.set("NeedAppearances", true);
            }
            if let Some(xfa) = self.xfa {
                acro.set("XFA", xfa);
            }
            if self.needs_rendering {
                acro.set("NeedsRendering", true);
            }
            let acro_id = self.doc.add_object(Object::Dictionary(acro));

            let mut catalog = Dictionary::new();
            catalog.set("Type", "Catalog");
            catalog.set("Pages", self.pages_id);
            catalog.set("AcroForm", acro_id);
            if let Some(id) = self.outlines {
                catalog.set("Outlines", id);
            }
            let catalog_id = self.doc.add_object(Object::Dictionary(catalog));
            self.doc.trailer.set("Root", catalog_id);
            if let Some(id) = self.info {
                self.doc.trailer.set("Info", id);
            }
            self.doc.save(path).expect("write form fixture");
        }
    }

    fn write_five_kind_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.push_merged_text("Name", "OldName", [72, 640, 240, 664], "OLD-NAME");
        f.push_checkbox("Agree", "Yes", false, [72, 600, 90, 618]);
        f.push_radio_group("Color", &["Red", "Blue"], "Red", [72, 560, 90, 578]);
        f.push_choice("City", "Ankara", &["Ankara", "Izmir", "Bursa"], true, [72, 500, 200, 524]);
        f.push_choice("Pets", "Cat", &["Cat", "Dog"], false, [72, 460, 200, 484]);
        f.push_pushbutton("Go", [72, 420, 120, 444]);
        f.push_sig("Sign", [72, 380, 180, 420]);
        f.save(path);
    }

    fn write_inherit_fixture(path: &Path) {
        let mut f = FormDoc::new();
        let ap = f.add_text_ap("PARENT-VAL");
        let mut leaf = Dictionary::new();
        leaf.set("T", Object::string_literal("Leaf"));
        leaf.set("Type", "Annot");
        leaf.set("Subtype", "Widget");
        leaf.set("Rect", box_obj([72, 640, 240, 664]));
        leaf.set("AP", ap);
        leaf.set("P", f.page_id);
        let leaf_id = f.doc.add_object(Object::Dictionary(leaf));
        f.annots.push(leaf_id.into());

        let mut parent = Dictionary::new();
        parent.set("FT", pdf_name("Tx"));
        parent.set("Ff", Object::Integer(FF_MULTILINE as i64));
        parent.set("T", Object::string_literal("Group"));
        parent.set("V", Object::string_literal("ParentVal"));
        parent.set("Kids", vec![leaf_id.into()]);
        let parent_id = f.doc.add_object(Object::Dictionary(parent));
        if let Ok(Object::Dictionary(d)) = f.doc.get_object_mut(leaf_id) {
            d.set("Parent", parent_id);
        }
        f.fields.push(parent_id.into());
        f.save(path);
    }

    fn write_direct_indirect_fixture(path: &Path) {
        let mut f = FormDoc::new();
        // Indirect terminal (normal ref in /Fields).
        f.push_merged_text("IndirectTx", "iv", [72, 640, 200, 664], "IV");

        // Non-terminal wrapper with two kids — wrapper must not be listed.
        let ap_a = f.add_text_ap("A");
        let ap_b = f.add_text_ap("B");
        let mut a = Dictionary::new();
        a.set("FT", pdf_name("Tx"));
        a.set("T", Object::string_literal("KidA"));
        a.set("V", Object::string_literal("a"));
        a.set("Type", "Annot");
        a.set("Subtype", "Widget");
        a.set("Rect", box_obj([72, 600, 200, 624]));
        a.set("AP", ap_a);
        a.set("P", f.page_id);
        let a_id = f.doc.add_object(Object::Dictionary(a));
        f.annots.push(a_id.into());

        let mut b = Dictionary::new();
        b.set("FT", pdf_name("Tx"));
        b.set("T", Object::string_literal("KidB"));
        b.set("V", Object::string_literal("b"));
        b.set("Type", "Annot");
        b.set("Subtype", "Widget");
        b.set("Rect", box_obj([72, 560, 200, 584]));
        b.set("AP", ap_b);
        b.set("P", f.page_id);
        let b_id = f.doc.add_object(Object::Dictionary(b));
        f.annots.push(b_id.into());

        let mut wrap = Dictionary::new();
        wrap.set("T", Object::string_literal("Wrap"));
        wrap.set("Kids", vec![a_id.into(), b_id.into()]);
        let wrap_id = f.doc.add_object(Object::Dictionary(wrap));
        if let Ok(Object::Dictionary(d)) = f.doc.get_object_mut(a_id) {
            d.set("Parent", wrap_id);
        }
        if let Ok(Object::Dictionary(d)) = f.doc.get_object_mut(b_id) {
            d.set("Parent", wrap_id);
        }
        f.fields.push(wrap_id.into());

        // Direct (inline) field dict sitting in /Fields — not a reference.
        let ap_d = f.add_text_ap("DIR");
        let mut direct = Dictionary::new();
        direct.set("FT", pdf_name("Tx"));
        direct.set("T", Object::string_literal("DirectTx"));
        direct.set("V", Object::string_literal("dv"));
        direct.set("Type", "Annot");
        direct.set("Subtype", "Widget");
        direct.set("Rect", box_obj([72, 520, 200, 544]));
        direct.set("AP", ap_d);
        direct.set("P", f.page_id);
        f.fields.push(Object::Dictionary(direct));
        f.save(path);
    }

    fn write_cycle_fixture(path: &Path) {
        let mut f = FormDoc::new();
        let a_id = f.doc.new_object_id();
        let b_id = f.doc.new_object_id();
        let mut a = Dictionary::new();
        a.set("T", Object::string_literal("A"));
        a.set("Kids", vec![b_id.into()]);
        let mut b = Dictionary::new();
        b.set("T", Object::string_literal("B"));
        b.set("Kids", vec![a_id.into()]);
        f.doc.objects.insert(a_id, Object::Dictionary(a));
        f.doc.objects.insert(b_id, Object::Dictionary(b));
        f.fields.push(a_id.into());
        f.save(path);
    }

    fn write_non_dict_kid_fixture(path: &Path) {
        let mut f = FormDoc::new();
        let mut parent = Dictionary::new();
        parent.set("T", Object::string_literal("Bad"));
        parent.set("Kids", vec![Object::Integer(42)]);
        let id = f.doc.add_object(Object::Dictionary(parent));
        f.fields.push(id.into());
        f.save(path);
    }

    fn write_missing_ft_fixture(path: &Path) {
        let mut f = FormDoc::new();
        let mut leaf = Dictionary::new();
        leaf.set("T", Object::string_literal("NoType"));
        leaf.set("Type", "Annot");
        leaf.set("Subtype", "Widget");
        leaf.set("Rect", box_obj([72, 640, 200, 664]));
        leaf.set("P", f.page_id);
        let leaf_id = f.doc.add_object(Object::Dictionary(leaf));
        f.annots.push(leaf_id.into());
        let mut parent = Dictionary::new();
        parent.set("T", Object::string_literal("Orphan"));
        parent.set("Kids", vec![leaf_id.into()]);
        let parent_id = f.doc.add_object(Object::Dictionary(parent));
        if let Ok(Object::Dictionary(d)) = f.doc.get_object_mut(leaf_id) {
            d.set("Parent", parent_id);
        }
        f.fields.push(parent_id.into());
        f.save(path);
    }

    fn write_broken_kids_fixture(path: &Path) {
        let mut f = FormDoc::new();
        let mut parent = Dictionary::new();
        parent.set("T", Object::string_literal("Broken"));
        parent.set("Kids", Object::string_literal("not-an-array"));
        let id = f.doc.add_object(Object::Dictionary(parent));
        f.fields.push(id.into());
        f.save(path);
    }

    fn write_xfa_fixture(path: &Path, as_array: bool, needs_rendering: bool) {
        let mut f = FormDoc::new();
        f.push_merged_text("Name", "Old", [72, 640, 200, 664], "OLD");
        if as_array {
            let pkt = f.doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                b"<xdp/>".to_vec(),
            )));
            f.xfa = Some(Object::Array(vec![
                Object::string_literal("xdp:xdp"),
                pkt.into(),
            ]));
        } else {
            let pkt = f.doc.add_object(Object::Stream(Stream::new(
                Dictionary::new(),
                b"<xdp/>".to_vec(),
            )));
            f.xfa = Some(pkt.into());
        }
        f.needs_rendering = needs_rendering;
        f.save(path);
    }

    fn write_agreed_checkbox(path: &Path) {
        let mut f = FormDoc::new();
        f.push_checkbox("Terms", "Agreed", false, [72, 600, 90, 618]);
        f.push_merged_text("Name", "Old", [72, 640, 200, 664], "OLD");
        f.save(path);
    }

    fn write_checked_yes_checkbox(path: &Path) {
        let mut f = FormDoc::new();
        f.push_checkbox("Agree", "Yes", true, [72, 600, 90, 618]);
        f.save(path);
    }

    fn write_name_only_ap_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.push_checkbox_name_only_ap("Agree", "Yes", false, [72, 600, 90, 618]);
        f.push_radio_name_only_ap("Size", &["S", "M"], "Off", [72, 560, 90, 578]);
        f.save(path);
    }

    fn write_skip_flags_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.push_merged_text("Open", "old-open", [72, 640, 200, 664], "OPEN");

        let ap_ro = f.add_text_ap("RO");
        let mut ro = Dictionary::new();
        ro.set("FT", pdf_name("Tx"));
        ro.set("Ff", Object::Integer(FF_READONLY as i64));
        ro.set("T", Object::string_literal("Locked"));
        ro.set("V", Object::string_literal("keep-ro"));
        ro.set("Type", "Annot");
        ro.set("Subtype", "Widget");
        ro.set("Rect", box_obj([72, 600, 200, 624]));
        ro.set("AP", ap_ro);
        ro.set("P", f.page_id);
        let ro_id = f.doc.add_object(Object::Dictionary(ro));
        f.fields.push(ro_id.into());
        f.annots.push(ro_id.into());

        for (name, flag, value, y) in [
            ("HiddenF", F_HIDDEN, "keep-hidden", 560i64),
            ("InvisibleF", F_INVISIBLE, "keep-inv", 520),
            ("NoViewF", F_NOVIEW, "keep-noview", 480),
        ] {
            let ap = f.add_text_ap(value);
            let mut d = Dictionary::new();
            d.set("FT", pdf_name("Tx"));
            d.set("T", Object::string_literal(name));
            d.set("V", Object::string_literal(value));
            d.set("F", Object::Integer(flag));
            d.set("Type", "Annot");
            d.set("Subtype", "Widget");
            d.set("Rect", box_obj([72, y, 200, y + 24]));
            d.set("AP", ap);
            d.set("P", f.page_id);
            let id = f.doc.add_object(Object::Dictionary(d));
            f.fields.push(id.into());
            f.annots.push(id.into());
        }
        f.save(path);
    }

    fn write_leftover_structure_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.with_catalog_extras();
        f.push_merged_text("Name", "OldName", [72, 640, 240, 664], "OLD-NAME");
        f.push_merged_text("Other", "leave-me", [72, 600, 240, 624], "OTHER");
        f.push_leftover_text();
        f.save(path);
    }

    fn write_flatten_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.with_catalog_extras();
        f.push_merged_text("Name", "OldName", [72, 640, 240, 664], "OLD-NAME");
        f.push_leftover_text();
        f.push_leftover_link();
        f.save(path);
    }

    fn write_rotate_crop_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.rotate = 90;
        f.crop = Some([72, 72, 540, 720]);
        f.push_merged_text("Name", "Old", [100, 200, 180, 240], "OLD");
        f.save(path);
    }

    fn write_stale_ap_fixture(path: &Path) {
        let mut f = FormDoc::new();
        f.need_appearances = true;
        f.push_merged_text("Name", "OldName", [72, 640, 240, 664], "STALE-OLD");
        f.save(path);
    }

    fn load(path: &Path) -> Document {
        Document::load(path).expect("load pdf")
    }

    fn catalog_has(doc: &Document, key: &[u8]) -> bool {
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        doc.get_dictionary(root).unwrap().get(key).is_ok()
    }

    fn first_page<'a>(doc: &'a Document) -> &'a Dictionary {
        let id = *doc.get_pages().get(&1).expect("page 1");
        doc.get_dictionary(id).expect("page dict")
    }

    fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
        match obj {
            Object::Dictionary(d) => Some(d),
            Object::Reference(id) => doc.get_dictionary(*id).ok(),
            _ => None,
        }
    }

    fn dict_t(d: &Dictionary) -> Option<String> {
        match d.get(b"T").ok()? {
            Object::String(b, _) => Some(decode_pdf_string(b)),
            _ => None,
        }
    }

    fn walk_named<'a>(
        doc: &'a Document,
        obj: &'a Object,
        want: &str,
        parent_name: &str,
        out: &mut Vec<&'a Dictionary>,
    ) {
        let Some(d) = resolve(doc, obj) else {
            return;
        };
        let name = match dict_t(d) {
            Some(t) if parent_name.is_empty() => t,
            Some(t) => format!("{parent_name}.{t}"),
            None => parent_name.to_string(),
        };
        if name == want || dict_t(d).as_deref() == Some(want) {
            out.push(d);
        }
        if let Ok(Object::Array(kids)) = d.get(b"Kids") {
            for kid in kids {
                walk_named(doc, kid, want, &name, out);
            }
        }
    }

    fn fields_named<'a>(doc: &'a Document, want: &str) -> Vec<&'a Dictionary> {
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let cat = doc.get_dictionary(root).unwrap();
        let acro = match cat.get(b"AcroForm").ok() {
            Some(Object::Reference(id)) => doc.get_dictionary(*id).ok(),
            Some(Object::Dictionary(d)) => Some(d),
            _ => None,
        };
        let Some(acro) = acro else {
            return Vec::new();
        };
        let Ok(Object::Array(fields)) = acro.get(b"Fields") else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for f in fields {
            walk_named(doc, f, want, "", &mut out);
        }
        out
    }

    fn field_v(doc: &Document, name: &str) -> Option<String> {
        let dicts = fields_named(doc, name);
        for d in dicts {
            if let Ok(v) = d.get(b"V") {
                return object_text(doc, v);
            }
        }
        None
    }

    fn field_v_bytes(doc: &Document, name: &str) -> Option<Vec<u8>> {
        let dicts = fields_named(doc, name);
        for d in dicts {
            if let Ok(Object::String(b, _)) = d.get(b"V") {
                return Some(b.clone());
            }
        }
        None
    }

    fn object_text(doc: &Document, obj: &Object) -> Option<String> {
        match obj {
            Object::String(b, _) => Some(decode_pdf_string(b)),
            Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
            Object::Reference(id) => object_text(doc, doc.get_object(*id).ok()?),
            _ => None,
        }
    }

    fn widget_as_names(doc: &Document, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        collect_as(doc, &fields_named(doc, name), &mut out);
        out
    }

    fn collect_as(doc: &Document, dicts: &[&Dictionary], out: &mut Vec<String>) {
        for d in dicts {
            if let Ok(Object::Name(n)) = d.get(b"AS") {
                out.push(String::from_utf8_lossy(n).into_owned());
            }
            if let Ok(Object::Array(kids)) = d.get(b"Kids") {
                let child_dicts: Vec<&Dictionary> =
                    kids.iter().filter_map(|k| resolve(doc, k)).collect();
                collect_as(doc, &child_dicts, out);
            }
        }
    }

    fn widget_ap_blobs(doc: &Document, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        collect_ap(doc, &fields_named(doc, name), &mut out);
        out
    }

    fn collect_ap(doc: &Document, dicts: &[&Dictionary], out: &mut Vec<String>) {
        for d in dicts {
            if let Ok(ap) = d.get(b"AP") {
                push_ap_blob(doc, ap, out);
            }
            if let Ok(Object::Array(kids)) = d.get(b"Kids") {
                let child_dicts: Vec<&Dictionary> =
                    kids.iter().filter_map(|k| resolve(doc, k)).collect();
                collect_ap(doc, &child_dicts, out);
            }
        }
    }

    fn push_ap_blob(doc: &Document, ap: &Object, out: &mut Vec<String>) {
        let dict = match ap {
            Object::Dictionary(d) => d,
            Object::Reference(id) => match doc.get_object(*id).ok() {
                Some(Object::Dictionary(d)) => d,
                Some(Object::Stream(s)) => {
                    out.push(stream_text(s));
                    return;
                }
                _ => return,
            },
            Object::Stream(s) => {
                out.push(stream_text(s));
                return;
            }
            _ => return,
        };
        if let Ok(n) = dict.get(b"N") {
            match n {
                Object::Reference(id) => {
                    if let Ok(Object::Stream(s)) = doc.get_object(*id) {
                        out.push(stream_text(s));
                    } else if let Ok(Object::Dictionary(states)) = doc.get_object(*id) {
                        for (_, v) in states.iter() {
                            if let Ok(id) = v.as_reference() {
                                if let Ok(Object::Stream(s)) = doc.get_object(id) {
                                    out.push(stream_text(s));
                                }
                            }
                        }
                    }
                }
                Object::Dictionary(states) => {
                    for (_, v) in states.iter() {
                        if let Ok(id) = v.as_reference() {
                            if let Ok(Object::Stream(s)) = doc.get_object(id) {
                                out.push(stream_text(s));
                            }
                        }
                    }
                }
                Object::Stream(s) => out.push(stream_text(s)),
                _ => {}
            }
        }
    }

    fn stream_text(s: &Stream) -> String {
        let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn annot_subtypes(doc: &Document) -> Vec<String> {
        let page = first_page(doc);
        let Ok(Object::Array(annots)) = page.get(b"Annots") else {
            return Vec::new();
        };
        annots
            .iter()
            .filter_map(|a| {
                let d = resolve(doc, a)?;
                match d.get(b"Subtype").ok()? {
                    Object::Name(n) => Some(String::from_utf8_lossy(n).into_owned()),
                    _ => None,
                }
            })
            .collect()
    }

    fn v(name: &str, value: &str) -> FormValue {
        FormValue {
            name: name.into(),
            value: value.into(),
        }
    }

    fn listed_by_name<'a>(fields: &'a [FormField], name: &str) -> Option<&'a FormField> {
        fields.iter().find(|f| f.name == name || f.name.ends_with(&format!(".{name}")))
    }

    fn deref_obj<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
        match obj {
            Object::Reference(id) => doc.get_object(*id).unwrap_or(obj),
            other => other,
        }
    }

    fn collect_field_dicts<'a>(
        doc: &'a Document,
        dicts: &[&'a Dictionary],
        out: &mut Vec<&'a Dictionary>,
    ) {
        for d in dicts {
            out.push(*d);
            if let Ok(Object::Array(kids)) = d.get(b"Kids") {
                let child: Vec<&Dictionary> = kids.iter().filter_map(|k| resolve(doc, k)).collect();
                collect_field_dicts(doc, &child, out);
            }
        }
    }

    fn ap_n_export<'a>(doc: &'a Document, field: &str, export: &str) -> Option<&'a Object> {
        let roots = fields_named(doc, field);
        let mut all = Vec::new();
        collect_field_dicts(doc, &roots, &mut all);
        for d in all {
            let Ok(ap_obj) = d.get(b"AP") else {
                continue;
            };
            let Object::Dictionary(ap_dict) = deref_obj(doc, ap_obj) else {
                continue;
            };
            let Ok(n_obj) = ap_dict.get(b"N") else {
                continue;
            };
            let Object::Dictionary(states) = deref_obj(doc, n_obj) else {
                continue;
            };
            let Ok(on) = states.get(export.as_bytes()) else {
                continue;
            };
            return Some(deref_obj(doc, on));
        }
        None
    }

    fn object_kind(obj: &Object) -> &'static str {
        match obj {
            Object::Null => "Null",
            Object::Boolean(_) => "Boolean",
            Object::Integer(_) => "Integer",
            Object::Real(_) => "Real",
            Object::Name(_) => "Name",
            Object::String(_, _) => "String",
            Object::Array(_) => "Array",
            Object::Dictionary(_) => "Dictionary",
            Object::Stream(_) => "Stream",
            Object::Reference(_) => "Reference",
        }
    }

    fn stream_has_path_op(content: &str) -> bool {
        content.split_whitespace().any(|tok| tok == "re" || tok == "m")
    }

    fn assert_ap_n_on_is_drawn_stream(doc: &Document, field: &str, export: &str) {
        match ap_n_export(doc, field, export) {
            Some(Object::Stream(s)) => {
                let text = stream_text(s);
                assert!(
                    stream_has_path_op(&text),
                    "F17: /AP /N /{export} must be a stream that draws (path ops re or m); got {text:?}"
                );
            }
            Some(other) => panic!(
                "F17: /AP /N /{export} must be a Form XObject stream (or Reference to Stream), not a {}; name-only /AP is invisible in Preview/Chrome",
                object_kind(other)
            ),
            None => panic!("F17: missing /AP /N /{export} after apply"),
        }
    }

    // --- F1 -----------------------------------------------------------------

    #[test]
    fn classify_tx_checkbox_radio_combo_list() {
        assert_eq!(
            classify_field(Some(b"Tx"), 0),
            Some(FormFieldKind::Text),
            "F1: /FT /Tx is text"
        );
        assert_eq!(
            classify_field(Some(b"Btn"), 0),
            Some(FormFieldKind::Checkbox),
            "F1: /FT /Btn without Radio/Pushbutton is checkbox"
        );
        assert_eq!(
            classify_field(Some(b"Btn"), FF_RADIO),
            Some(FormFieldKind::Radio),
            "F1: /FT /Btn + Radio bit is radio"
        );
        assert_eq!(
            classify_field(Some(b"Ch"), FF_COMBO),
            Some(FormFieldKind::Combo),
            "F1: /FT /Ch + Combo bit is combo"
        );
        assert_eq!(
            classify_field(Some(b"Ch"), 0),
            Some(FormFieldKind::List),
            "F1: /FT /Ch without Combo is list"
        );
        assert_eq!(
            classify_field(Some(b"Btn"), FF_PUSHBUTTON),
            None,
            "F1: pushbutton is not a fillable kind"
        );
        assert_eq!(
            classify_field(Some(b"Sig"), 0),
            None,
            "F1: /FT /Sig is skipped, not listed"
        );
    }

    #[test]
    fn classify_combo_vs_list_by_ff_bit() {
        assert_eq!(
            classify_field(Some(b"Ch"), FF_COMBO | FF_EDIT),
            Some(FormFieldKind::Combo),
            "extra: Combo bit, not widget count, classifies combo"
        );
        assert_eq!(
            classify_field(Some(b"Ch"), 0),
            Some(FormFieldKind::List),
            "extra: /Ch without Combo is list"
        );
    }

    #[test]
    fn list_five_kinds_not_pushbutton_or_sig() {
        let scratch = Scratch::new("f1-five");
        let src = scratch.pdf("src.pdf");
        write_five_kind_fixture(&src);
        let fields = list_form_fields(src.to_str().unwrap())
            .expect("F1: list_form_fields must not error on a well-formed tree");
        let kinds = kinds_of(&fields);
        let names = names_of(&fields);
        assert!(
            kinds.contains(&FormFieldKind::Text),
            "F1: list must include text; got names={names:?} kinds={kinds:?}"
        );
        assert!(
            kinds.contains(&FormFieldKind::Checkbox),
            "F1: list must include checkbox; got names={names:?} kinds={kinds:?}"
        );
        assert!(
            kinds.contains(&FormFieldKind::Radio),
            "F1: list must include radio; got names={names:?} kinds={kinds:?}"
        );
        assert!(
            kinds.contains(&FormFieldKind::Combo),
            "F1: list must include combo; got names={names:?} kinds={kinds:?}"
        );
        assert!(
            kinds.contains(&FormFieldKind::List),
            "F1: list must include list; got names={names:?} kinds={kinds:?}"
        );
        assert!(
            !names.iter().any(|n| n == "Go"),
            "F1: pushbutton must not be listed; got {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "Sign"),
            "F1: /FT /Sig must not be listed; got {names:?}"
        );
        assert_eq!(
            kinds.iter().filter(|k| **k == FormFieldKind::Radio).count(),
            1,
            "F1 extra: radio group listed once, not per widget; kinds={kinds:?}"
        );
    }

    // --- F2 -----------------------------------------------------------------

    #[test]
    fn list_inherits_ft_ff_v_from_parent() {
        let scratch = Scratch::new("f2-inherit");
        let src = scratch.pdf("src.pdf");
        write_inherit_fixture(&src);
        let fields = list_form_fields(src.to_str().unwrap())
            .expect("F2: inherit fixture must list, not error");
        let leaf = listed_by_name(&fields, "Leaf")
            .unwrap_or_else(|| panic!("F2: child missing /FT must still be listed; got {:?}", names_of(&fields)));
        assert_eq!(
            leaf.kind,
            FormFieldKind::Text,
            "F2: inherited /FT /Tx must classify as text"
        );
        assert_eq!(
            leaf.value.as_deref(),
            Some("ParentVal"),
            "F2: inherited /V must be the listed value"
        );
        assert!(
            leaf.multiline,
            "F2: inherited /Ff Multiline must surface on the terminal"
        );
        assert!(
            fields.iter().all(|f| f.kind != FormFieldKind::Text || f.name.contains("Leaf") || f.name.contains("Group")),
            "F2: non-terminal parent must not be a second fillable text control; got {:?}",
            names_of(&fields)
        );
    }

    // --- F3 -----------------------------------------------------------------

    #[test]
    fn list_direct_and_indirect_once_skips_nonterminals() {
        let scratch = Scratch::new("f3-direct");
        let src = scratch.pdf("src.pdf");
        write_direct_indirect_fixture(&src);
        let fields = list_form_fields(src.to_str().unwrap())
            .expect("F3: mixed direct/indirect tree must list");
        let names = names_of(&fields);
        assert!(
            names.iter().any(|n| n == "IndirectTx" || n.ends_with(".IndirectTx")),
            "F3: indirect field must appear once; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "DirectTx" || n.ends_with(".DirectTx")),
            "F3: direct (inline dict) field must appear once; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "KidA" || n.ends_with(".KidA")),
            "F3: terminal kid A must be listed; got {names:?}"
        );
        assert!(
            names.iter().any(|n| n == "KidB" || n.ends_with(".KidB")),
            "F3: terminal kid B must be listed; got {names:?}"
        );
        assert!(
            !names.iter().any(|n| n == "Wrap"),
            "F3: non-terminal parent must not be listed as a fillable control; got {names:?}"
        );
        assert_eq!(
            names
                .iter()
                .filter(|n| n.ends_with("IndirectTx") || *n == "IndirectTx")
                .count(),
            1,
            "F3: indirect field listed more than once; {names:?}"
        );
    }

    // --- F4 -----------------------------------------------------------------

    fn assert_malformed(path: &Path, ctx: &str) {
        let listed = std::panic::catch_unwind(|| list_form_fields(path.to_str().unwrap()))
            .unwrap_or_else(|_| panic!("{ctx}: list_form_fields must not panic"));
        let err = match listed {
            Ok(fields) => panic!(
                "{ctx}: malformed tree must be AppError, not Ok({} fields)",
                fields.len()
            ),
            Err(e) => e,
        };
        assert_ne!(
            err.code, "ENGINE_FAILED",
            "{ctx}: malformed tree must be a specific AppError, not ENGINE_FAILED"
        );
        assert_actionable(&err, "MALFORMED_FORM", ctx);
    }

    #[test]
    fn list_cycle_is_app_error_not_empty() {
        let scratch = Scratch::new("f4-cycle");
        let src = scratch.pdf("src.pdf");
        write_cycle_fixture(&src);
        assert_malformed(&src, "F4: cyclic /Kids");
    }

    #[test]
    fn list_non_dict_kid_is_app_error() {
        let scratch = Scratch::new("f4-nondict");
        let src = scratch.pdf("src.pdf");
        write_non_dict_kid_fixture(&src);
        assert_malformed(&src, "F4: non-dict kid");
    }

    #[test]
    fn list_missing_ft_after_inherit_is_app_error() {
        let scratch = Scratch::new("f4-noft");
        let src = scratch.pdf("src.pdf");
        write_missing_ft_fixture(&src);
        assert_malformed(&src, "F4: missing /FT after inherit");
    }

    #[test]
    fn list_broken_kids_is_app_error() {
        let scratch = Scratch::new("f4-kids");
        let src = scratch.pdf("src.pdf");
        write_broken_kids_fixture(&src);
        assert_malformed(&src, "F4: /Kids not an array");
    }

    // --- F5 -----------------------------------------------------------------

    #[test]
    fn detect_xfa_stream_is_unsupported() {
        let scratch = Scratch::new("f5-stream");
        let src = scratch.pdf("src.pdf");
        write_xfa_fixture(&src, false, false);
        let err = detect_xfa(src.to_str().unwrap())
            .expect_err("F5: detect_xfa on /XFA stream must be UNSUPPORTED_XFA");
        assert_actionable(&err, "UNSUPPORTED_XFA", "F5: XFA stream");
    }

    #[test]
    fn detect_xfa_array_is_unsupported() {
        let scratch = Scratch::new("f5-array");
        let src = scratch.pdf("src.pdf");
        write_xfa_fixture(&src, true, false);
        let err = detect_xfa(src.to_str().unwrap())
            .expect_err("F5: detect_xfa on /XFA array must be UNSUPPORTED_XFA");
        assert_actionable(&err, "UNSUPPORTED_XFA", "F5: XFA array");
    }

    #[test]
    fn list_xfa_is_unsupported_xfa_not_empty() {
        let scratch = Scratch::new("f5-list");
        let src = scratch.pdf("src.pdf");
        write_xfa_fixture(&src, false, false);
        let result = list_form_fields(src.to_str().unwrap());
        let err = match result {
            Ok(fields) => panic!(
                "F5: XFA must not list as empty or filled; got Ok({} fields)",
                fields.len()
            ),
            Err(e) => e,
        };
        assert_actionable(&err, "UNSUPPORTED_XFA", "F5: list XFA");
    }

    #[test]
    fn apply_xfa_is_unsupported_xfa_dest_unchanged() {
        let scratch = Scratch::new("f5-apply");
        let dest = scratch.pdf("dest.pdf");
        write_xfa_fixture(&dest, false, true);
        let before = std::fs::read(&dest).unwrap();
        let err = apply_form_values(&dest.to_string_lossy(), &[v("Name", "Ada")], false)
            .expect_err("F5: apply on XFA must be UNSUPPORTED_XFA");
        assert_actionable(&err, "UNSUPPORTED_XFA", "F5: apply XFA");
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            before,
            "F5: dest bytes must be unchanged after XFA reject"
        );
    }

    // --- F6 -----------------------------------------------------------------

    #[test]
    fn apply_writes_v_and_updates_ap() {
        let scratch = Scratch::new("f6-apply");
        let dest = scratch.pdf("dest.pdf");
        write_five_kind_fixture(&dest);
        apply_form_values(
            dest.to_str().unwrap(),
            &[
                v("Name", "Ada"),
                v("Agree", "Yes"),
                v("Color", "Blue"),
                v("City", "Izmir"),
                v("Pets", "Dog"),
            ],
            false,
        )
        .expect("F6: apply on a well-formed form must succeed");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Name").as_deref(),
            Some("Ada"),
            "F6: dest text /V must match the session"
        );
        let aps = widget_ap_blobs(&doc, "Name");
        assert!(
            !aps.is_empty(),
            "F6: text widget must have /AP after fill"
        );
        assert!(
            aps.iter().any(|b| !b.contains("OLD-NAME")),
            "F6: /V-only without appearance update fails this ID; stale AP still {:?}",
            aps
        );
        assert_eq!(
            field_v(&doc, "Agree").as_deref(),
            Some("Yes"),
            "F6: checkbox /V must match the session"
        );
        assert!(
            widget_as_names(&doc, "Agree").iter().any(|s| s == "Yes"),
            "F6: checkbox /AS must follow the on-state; got {:?}",
            widget_as_names(&doc, "Agree")
        );
        assert_eq!(
            field_v(&doc, "Color").as_deref(),
            Some("Blue"),
            "F6: radio parent /V must match the session"
        );
        assert_eq!(
            field_v(&doc, "City").as_deref(),
            Some("Izmir"),
            "F6: combo /V must match the session"
        );
        assert_eq!(
            field_v(&doc, "Pets").as_deref(),
            Some("Dog"),
            "F6: list /V must match the session"
        );
    }

    // --- F7 -----------------------------------------------------------------

    #[test]
    fn apply_checkbox_export_yes() {
        let scratch = Scratch::new("f7-yes");
        let dest = scratch.pdf("dest.pdf");
        write_five_kind_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Agree", "Yes")], false)
            .expect("F7: apply checkbox Yes");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Agree").as_deref(),
            Some("Yes"),
            "F7: on-state /V is the /AP /N name that is not /Off"
        );
        assert!(
            widget_as_names(&doc, "Agree").iter().any(|s| s == "Yes"),
            "F7: /AS must be /Yes; got {:?}",
            widget_as_names(&doc, "Agree")
        );
    }

    #[test]
    fn apply_checkbox_export_agreed() {
        let scratch = Scratch::new("f7-agreed");
        let dest = scratch.pdf("dest.pdf");
        write_agreed_checkbox(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Terms", "Agreed")], false)
            .expect("F7: apply checkbox Agreed");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Terms").as_deref(),
            Some("Agreed"),
            "F7: must not hard-code /Yes; export name is /Agreed"
        );
        assert!(
            widget_as_names(&doc, "Terms")
                .iter()
                .any(|s| s == "Agreed"),
            "F7: /AS must be /Agreed; got {:?}",
            widget_as_names(&doc, "Terms")
        );
        assert!(
            widget_as_names(&doc, "Terms").iter().all(|s| s != "Yes"),
            "F7: /AS must not be hard-coded /Yes when export is /Agreed"
        );
    }

    #[test]
    fn apply_checkbox_off_is_off() {
        let scratch = Scratch::new("f7-off");
        let dest = scratch.pdf("dest.pdf");
        write_checked_yes_checkbox(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Agree", "Off")], false)
            .expect("F7: apply checkbox Off");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Agree").as_deref(),
            Some("Off"),
            "F7: off-state /V is /Off"
        );
        assert!(
            widget_as_names(&doc, "Agree").iter().any(|s| s == "Off"),
            "F7: off-state /AS is /Off; got {:?}",
            widget_as_names(&doc, "Agree")
        );
    }

    // --- F8 -----------------------------------------------------------------

    #[test]
    fn apply_radio_parent_v_and_sibling_as() {
        let scratch = Scratch::new("f8-radio");
        let dest = scratch.pdf("dest.pdf");
        write_five_kind_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Color", "Blue")], false)
            .expect("F8: apply radio");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Color").as_deref(),
            Some("Blue"),
            "F8: parent /V is the selected child's export name"
        );
        let as_names = widget_as_names(&doc, "Color");
        assert!(
            as_names.iter().any(|s| s == "Blue"),
            "F8: selected widget /AS must be /Blue; got {as_names:?}"
        );
        assert!(
            as_names.iter().any(|s| s == "Off"),
            "F8: unselected sibling /AS must be /Off, not left on the old /AS; got {as_names:?}"
        );
        assert!(
            as_names.iter().all(|s| s != "Red"),
            "F8: no widget may remain on the old /Red /AS; got {as_names:?}"
        );
    }

    // --- F9 -----------------------------------------------------------------

    #[test]
    fn apply_skips_readonly_hidden_writes_others() {
        let scratch = Scratch::new("f9-skip");
        let dest = scratch.pdf("dest.pdf");
        write_skip_flags_fixture(&dest);
        apply_form_values(
            dest.to_str().unwrap(),
            &[
                v("Open", "new-open"),
                v("Locked", "hacked-ro"),
                v("HiddenF", "hacked-hidden"),
                v("InvisibleF", "hacked-inv"),
                v("NoViewF", "hacked-noview"),
            ],
            false,
        )
        .expect("F9: apply must succeed and skip protected fields");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Locked").as_deref(),
            Some("keep-ro"),
            "F9: ReadOnly /V must stay equal to source"
        );
        assert_eq!(
            field_v(&doc, "HiddenF").as_deref(),
            Some("keep-hidden"),
            "F9: Hidden widget /V must stay equal to source"
        );
        assert_eq!(
            field_v(&doc, "InvisibleF").as_deref(),
            Some("keep-inv"),
            "F9: Invisible widget /V must stay equal to source"
        );
        assert_eq!(
            field_v(&doc, "NoViewF").as_deref(),
            Some("keep-noview"),
            "F9: NoView widget /V must stay equal to source"
        );
        assert_eq!(
            field_v(&doc, "Open").as_deref(),
            Some("new-open"),
            "F9: other fields in the same file must still write"
        );
    }

    // --- F10 ----------------------------------------------------------------

    #[test]
    fn apply_turkish_v_utf16be_and_ap_not_question_mark() {
        let scratch = Scratch::new("f10-tr");
        let dest = scratch.pdf("dest.pdf");
        write_five_kind_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Name", "GİZLİ Şğı")], false)
            .expect("F10: Turkish is handled, not rejected");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Name").as_deref(),
            Some("GİZLİ Şğı"),
            "F10: dest /V must round-trip GİZLİ Şğı (not ?)"
        );
        let raw = field_v_bytes(&doc, "Name").unwrap_or_default();
        assert!(
            raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF,
            "F10: non-ASCII /V must be UTF-16BE+BOM via encode_pdf_string; got {raw:?}"
        );
        let aps = widget_ap_blobs(&doc, "Name");
        assert!(
            !aps.is_empty(),
            "F10: Noto /AP must be written for Turkish text"
        );
        assert!(
            aps.iter().all(|b| !b.contains("(?)") && !b.contains("G????")),
            "F10: /AP must not substitute ? for Turkish; got {aps:?}"
        );
        assert!(
            aps.iter().any(|b| !b.contains("OLD-NAME")),
            "F10: /AP must be regenerated, not the stale ASCII appearance; got {aps:?}"
        );
    }

    // --- F11 ----------------------------------------------------------------

    #[test]
    fn apply_keeps_leftover_text_catalog_and_unrelated_widget() {
        let scratch = Scratch::new("f11-left");
        let dest = scratch.pdf("dest.pdf");
        write_leftover_structure_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Name", "Ada")], false)
            .expect("F11: form write must succeed");
        let doc = load(&dest);
        assert!(
            catalog_has(&doc, b"AcroForm"),
            "F11: catalog /AcroForm must survive form write"
        );
        assert!(
            catalog_has(&doc, b"Outlines"),
            "F11: catalog Outlines must survive form write"
        );
        assert!(
            doc.trailer.get(b"Info").is_ok(),
            "F11: Info must survive form write"
        );
        let subtypes = annot_subtypes(&doc);
        assert!(
            subtypes.iter().any(|s| s == "Text"),
            "F11: leftover /Subtype /Text must survive form write; got {subtypes:?}"
        );
        assert_eq!(
            field_v(&doc, "Other").as_deref(),
            Some("leave-me"),
            "F11: unrelated widget not in the write set must be left alone"
        );
        assert_eq!(
            field_v(&doc, "Name").as_deref(),
            Some("Ada"),
            "F11: written field still updates (form-apply lock, not overlay-only)"
        );
    }

    // --- F12 ----------------------------------------------------------------

    #[test]
    fn apply_interactive_keeps_acroform() {
        let scratch = Scratch::new("f12-int");
        let dest = scratch.pdf("dest.pdf");
        write_flatten_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Name", "Ada")], false)
            .expect("F12: interactive apply");
        let doc = load(&dest);
        assert!(
            catalog_has(&doc, b"AcroForm"),
            "F12: interactive dest still has catalog /AcroForm if the source did"
        );
        assert_eq!(
            field_v(&doc, "Name").as_deref(),
            Some("Ada"),
            "F12: interactive fill still writes /V"
        );
    }

    #[test]
    fn apply_flatten_widgets_only_keeps_text_and_link() {
        let scratch = Scratch::new("f12-flat");
        let dest = scratch.pdf("dest.pdf");
        write_flatten_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Name", "Ada")], true)
            .expect("F12: flatten-on apply");
        let doc = load(&dest);
        let subtypes = annot_subtypes(&doc);
        assert!(
            !subtypes.iter().any(|s| s == "Widget"),
            "F12: flatten is widgets only — dest /Annots must drop /Widget; got {subtypes:?}"
        );
        assert!(
            subtypes.iter().any(|s| s == "Text"),
            "F12: leftover /Text must survive widget-only flatten; got {subtypes:?}"
        );
        assert!(
            subtypes.iter().any(|s| s == "Link"),
            "F12: leftover /Link must survive widget-only flatten; got {subtypes:?}"
        );
        let blob = {
            let mut d = load(&dest);
            let _ = d.decompress();
            let mut out = String::new();
            for obj in d.objects.values() {
                if let Object::Stream(s) = obj {
                    let bytes = s.get_plain_content().unwrap_or_else(|_| s.content.clone());
                    out.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            out
        };
        assert!(
            blob.contains("Hello"),
            "F12: source page stream must survive flatten; blob={blob:?}"
        );
    }

    #[test]
    fn flatten_snapshot_acro_form_matches_dest() {
        let scratch = Scratch::new("f12-snap");
        let dest = scratch.pdf("dest.pdf");
        write_flatten_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Name", "Ada")], true)
            .expect("F12: flatten-on for snapshot honesty");
        let doc = load(&dest);
        let flags = catalog_flags_from_doc(&doc);
        let has_key = catalog_has(&doc, b"AcroForm");
        assert_eq!(
            flags.acro_form, has_key,
            "F12: snapshot.catalog.acro_form must equal whether dest still has the key"
        );
    }

    // --- F13 ----------------------------------------------------------------

    #[test]
    fn list_rect_unrotated_on_rotate_90_crop() {
        let scratch = Scratch::new("f13-rect");
        let src = scratch.pdf("src.pdf");
        write_rotate_crop_fixture(&src);
        let fields = list_form_fields(src.to_str().unwrap())
            .expect("F13: rotate+crop widget must list");
        let field = listed_by_name(&fields, "Name")
            .unwrap_or_else(|| panic!("F13: widget must be listed; got {:?}", names_of(&fields)));
        let rect = field
            .rect
            .as_ref()
            .unwrap_or_else(|| panic!("F13: widget with /Rect must have listed geometry"));
        assert!(
            (rect.x - 100.0).abs() < 0.5
                && (rect.y - 200.0).abs() < 0.5
                && (rect.w - 80.0).abs() < 0.5
                && (rect.h - 40.0).abs() < 0.5,
            "F13: listed rect must be unrotated {{x:100,y:200,w:80,h:40}}, not display-swapped; got {rect:?}"
        );
        assert_eq!(
            field.page_index,
            Some(0),
            "F13: assembled pageIndex is 0"
        );
    }

    // --- F14 ----------------------------------------------------------------

    #[test]
    fn form_only_save_is_not_no_edits() {
        let values = [v("Name", "Ada")];
        assert!(
            has_edits(0, &values),
            "F14: form values and zero stamps must be saveable, not NO_EDITS"
        );
        assert!(
            !has_edits(0, &[]),
            "F14: neither stamps nor form values stays NO_EDITS"
        );
        assert!(
            has_edits(1, &[]),
            "F14: overlay stamps without form values remain saveable"
        );
    }

    // --- F15 ----------------------------------------------------------------

    #[test]
    fn apply_replaces_stale_ap_for_new_v() {
        let scratch = Scratch::new("f15-stale");
        let dest = scratch.pdf("dest.pdf");
        write_stale_ap_fixture(&dest);
        apply_form_values(dest.to_str().unwrap(), &[v("Name", "Ada")], false)
            .expect("F15: apply on NeedAppearances + stale AP");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Name").as_deref(),
            Some("Ada"),
            "F15: dest /V must be the new value"
        );
        let aps = widget_ap_blobs(&doc, "Name");
        let stale = aps.iter().any(|b| b.contains("STALE-OLD"));
        let has_new = aps.iter().any(|b| !b.contains("STALE-OLD"));
        let root = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
        let cat = doc.get_dictionary(root).unwrap();
        let acro = match cat.get(b"AcroForm").unwrap() {
            Object::Reference(id) => doc.get_dictionary(*id).unwrap(),
            Object::Dictionary(d) => d,
            other => panic!("F15: AcroForm must be a dict, got {other:?}"),
        };
        let need = match acro.get(b"NeedAppearances") {
            Ok(Object::Boolean(b)) => *b,
            Ok(Object::Integer(i)) => *i != 0,
            _ => false,
        };
        assert!(
            has_new && (!stale || !need),
            "F15: stale /AP + new /V fails this ID; AP={aps:?} NeedAppearances={need}"
        );
        assert!(
            !aps.is_empty(),
            "F15: dest must have an appearance for the new value"
        );
    }

    // --- F16 ----------------------------------------------------------------

    #[test]
    fn fill_pdf_form_publishes_dest_through_gate() {
        let scratch = Scratch::new("f16-pub");
        let src = scratch.pdf("src.pdf");
        let dest = scratch.pdf("out.pdf");
        write_five_kind_fixture(&src);
        let src_before = std::fs::read(&src).unwrap();
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        fill_pdf_form(
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
            &[v("Name", "Ada")],
            false,
        )
        .expect("F16: form path must publish through #34");
        let bytes = std::fs::read(&dest).unwrap();
        assert_ne!(
            bytes.as_slice(),
            b"OLD-DEST",
            "F16: form path must publish dest (through validate_staged_pdf), not leave OLD-DEST"
        );
        assert!(
            bytes.starts_with(b"%PDF"),
            "F16: dest must be a PDF, got {} bytes",
            bytes.len()
        );
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Name").as_deref(),
            Some("Ada"),
            "F16: published dest /V must match the session"
        );
        assert_eq!(
            std::fs::read(&src).unwrap(),
            src_before,
            "F16: original file is never overwritten"
        );
    }

    #[test]
    fn fill_pdf_form_hard_link_is_overwrite() {
        let scratch = Scratch::new("f16-hl");
        let src = scratch.pdf("src.pdf");
        let dest = scratch.pdf("alias.pdf");
        write_five_kind_fixture(&src);
        if std::fs::hard_link(&src, &dest).is_err() {
            return;
        }
        let before = std::fs::read(&src).unwrap();
        let err = fill_pdf_form(
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
            &[v("Name", "Ada")],
            false,
        )
        .expect_err("F16: hard-linked dest must be OVERWRITE");
        assert_eq!(err.code, "OVERWRITE", "F16: hard-link must be OVERWRITE");
        assert_eq!(std::fs::read(&src).unwrap(), before);
    }

    #[test]
    fn form_apply_then_fatal_validate_leaves_dest() {
        // Gate the form path already uses: apply on sibling tmp, then #34.
        let scratch = Scratch::new("f16-fatal");
        let dest = scratch.pdf("out.pdf");
        let staged = scratch.pdf(".offpdf-form.pdf.tmp");
        write_five_kind_fixture(&staged);
        std::fs::write(&dest, b"OLD-DEST").unwrap();
        apply_form_values(staged.to_str().unwrap(), &[v("Name", "Ada")], false)
            .expect("apply on staged tmp");
        std::fs::write(&staged, b"%PDF-1.4\n%% truncated").unwrap();
        let result = validate_staged_pdf(&staged, &letter_snapshot(), None, |_args| {
            Ok((2, "qpdf --check: file is damaged".into()))
        });
        assert!(
            !staged.exists(),
            "F16: fatal qpdf --check must delete staged .offpdf-*.pdf.tmp"
        );
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            b"OLD-DEST",
            "F16: dest bytes must stay OLD-DEST"
        );
        let err = result.expect_err("F16: fatal check is INVALID_OUTPUT");
        assert_eq!(err.code, "INVALID_OUTPUT");
    }

    // --- F17 ----------------------------------------------------------------

    #[test]
    fn apply_name_only_ap_on_state_is_drawn_stream() {
        let scratch = Scratch::new("f17-name-ap");
        let dest = scratch.pdf("dest.pdf");
        write_name_only_ap_fixture(&dest);
        apply_form_values(
            dest.to_str().unwrap(),
            &[v("Agree", "Yes"), v("Size", "M")],
            false,
        )
        .expect("F17: apply on name-only /AP fixture must succeed");
        let doc = load(&dest);
        assert_eq!(
            field_v(&doc, "Agree").as_deref(),
            Some("Yes"),
            "F17: checkbox /V is written"
        );
        assert!(
            widget_as_names(&doc, "Agree").iter().any(|s| s == "Yes"),
            "F17: checkbox /AS is written; got {:?}",
            widget_as_names(&doc, "Agree")
        );
        assert_eq!(
            field_v(&doc, "Size").as_deref(),
            Some("M"),
            "F17: radio parent /V is written"
        );
        assert!(
            widget_as_names(&doc, "Size").iter().any(|s| s == "M"),
            "F17: radio /AS is written; got {:?}",
            widget_as_names(&doc, "Size")
        );
        assert_ap_n_on_is_drawn_stream(&doc, "Agree", "Yes");
        assert_ap_n_on_is_drawn_stream(&doc, "Size", "M");
    }
}
