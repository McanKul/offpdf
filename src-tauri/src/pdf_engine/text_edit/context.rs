//! Snapshot context (SPEC §B.10): one parsed snapshot, its font cache and the lazily computed
//! reference / page-tree counts every page walk of that snapshot shares, plus the resource
//! lookups the walker uses. Unresolvable references are reported as such (`Lookup::Broken`) so
//! the walker can refuse the page (§A.4); a font name that is simply absent stays the run-level
//! `MISSING_FONT`.

use crate::error::AppError;
use crate::pdf_engine::text_edit::content::{KidsCounts, RefCounts};
use crate::pdf_engine::text_edit::fonts::{FontCache, FontKey};
use crate::pdf_engine::text_edit::limits::PAGE_TREE_DEPTH_MAX;
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::snapshot::SourceSnapshot;
use crate::pdf_engine::text_edit::structure::OcConfig;
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use std::collections::HashSet;
use std::sync::OnceLock;

/// References followed for one value (a reference to a reference …).
const REF_HOPS_MAX: usize = 16;

pub struct SnapshotContext {
    pub snap: SourceSnapshot,
    pub fonts: FontCache,
    refs: OnceLock<RefCounts>,
    kids: OnceLock<Result<KidsCounts, TextReason>>,
    oc: OnceLock<OcConfig>,
}

impl SnapshotContext {
    pub fn new(snap: SourceSnapshot) -> Self {
        SnapshotContext {
            snap,
            fonts: FontCache::new(),
            refs: OnceLock::new(),
            kids: OnceLock::new(),
            oc: OnceLock::new(),
        }
    }

    pub fn doc(&self) -> &Document {
        &self.snap.doc
    }

    pub fn refs(&self) -> &RefCounts {
        self.refs.get_or_init(|| RefCounts::of(&self.snap.doc))
    }

    pub fn kids(&self) -> Result<&KidsCounts, TextReason> {
        self.kids
            .get_or_init(|| KidsCounts::of(&self.snap.doc))
            .as_ref()
            .map_err(|r| *r)
    }

    /// The catalog's default optional-content configuration, read on first use.
    pub fn oc_config(&self) -> &OcConfig {
        self.oc.get_or_init(|| OcConfig::of(&self.snap.doc))
    }

    /// The object id of the 0-based page `page_index`; `INVALID_PAGES` when there is none.
    pub fn page_id(&self, page_index: u32) -> Result<ObjectId, AppError> {
        usize::try_from(page_index)
            .ok()
            .and_then(|i| self.snap.pages.get(i).copied())
            .ok_or_else(|| invalid_page(page_index, self.snap.pages.len()))
    }
}

/// `INVALID_PAGES` for a page index the document does not have.
pub fn invalid_page(page_index: u32, page_count: usize) -> AppError {
    AppError::new(
        "INVALID_PAGES",
        "Page not found",
        format!(
            "This PDF has {page_count} page(s), so page {} does not exist.",
            u64::from(page_index) + 1
        ),
    )
    .with_suggestion("Reopen the file and try again.")
    .with_details(format!("page_index={page_index} pages={page_count}"))
}

/// Follows references (≤ 16 hops). `None` when one dangles. Returns the id of the last object
/// reached through a reference, if any.
pub fn resolve<'a>(doc: &'a Document, obj: &'a Object) -> Option<(Option<ObjectId>, &'a Object)> {
    let mut obj = obj;
    let mut id = None;
    for _ in 0..REF_HOPS_MAX {
        match obj {
            Object::Reference(r) => {
                id = Some(*r);
                obj = doc.objects.get(r)?;
            }
            other => return Some((id, other)),
        }
    }
    None
}

/// The result of looking a name up in a resource category.
pub enum Lookup<T> {
    Found(T),
    /// The category or the name does not exist.
    Missing,
    /// A reference on the way does not resolve, or a value has the wrong type.
    Broken(&'static str),
}

/// A resource dictionary in force and the object that holds it (the page, an ancestor of the
/// page, a Form XObject, or the resources dictionary itself when it is indirect).
#[derive(Clone, Copy)]
pub struct Res<'a> {
    pub owner: ObjectId,
    pub dict: Option<&'a Dictionary>,
    /// The dictionary came from an ancestor of the page (`/Resources` inherited from `/Pages`).
    pub inherited: bool,
    /// The dictionary is an indirect object (its id), for reference counting.
    pub dict_id: Option<ObjectId>,
}

/// A resolved category entry: its key (for fonts), the object id when indirect, the value, and
/// whether the category dictionary is an indirect object (its id).
pub struct Entry<'a> {
    pub id: Option<ObjectId>,
    pub value: &'a Object,
    pub category_id: Option<ObjectId>,
}

impl<'a> Res<'a> {
    pub fn empty(owner: ObjectId) -> Res<'a> {
        Res {
            owner,
            dict: None,
            inherited: false,
            dict_id: None,
        }
    }

    /// The page's `/Resources`, inherited through `/Parent` (nearest wins). A dangling or
    /// non-dictionary value is `Err` (page `MALFORMED_CONTENT`).
    pub fn of_page(doc: &'a Document, page_id: ObjectId) -> Result<Res<'a>, &'static str> {
        let mut cur = Some(page_id);
        let mut seen = HashSet::new();
        while let Some(id) = cur {
            if !seen.insert(id) || seen.len() > PAGE_TREE_DEPTH_MAX + 1 {
                return Err("page tree cycle");
            }
            let Some(Object::Dictionary(node)) = doc.objects.get(&id) else {
                return Err("page tree node");
            };
            if let Ok(value) = node.get(b"Resources") {
                return Res::from_value(doc, id, value, id != page_id);
            }
            cur = match node.get(b"Parent").ok() {
                None => None,
                Some(Object::Reference(p)) => Some(*p),
                Some(_) => return Err("page /Parent"),
            };
        }
        Ok(Res::empty(page_id))
    }

    /// A Form XObject's `/Resources`; `None` when the form has none (the caller inherits).
    pub fn of_form(
        doc: &'a Document,
        form_id: ObjectId,
        form: &'a Stream,
    ) -> Option<Result<Res<'a>, &'static str>> {
        let value = form.dict.get(b"Resources").ok()?;
        Some(Res::from_value(doc, form_id, value, false))
    }

    fn from_value(
        doc: &'a Document,
        owner: ObjectId,
        value: &'a Object,
        inherited: bool,
    ) -> Result<Res<'a>, &'static str> {
        match resolve(doc, value) {
            Some((id, Object::Dictionary(d))) => Ok(Res {
                owner: id.unwrap_or(owner),
                dict: Some(d),
                inherited,
                dict_id: id,
            }),
            Some((_, Object::Null)) => Ok(Res {
                owner,
                dict: None,
                inherited,
                dict_id: None,
            }),
            _ => Err("/Resources"),
        }
    }

    /// `category[name]`, resolved.
    pub fn entry(&self, doc: &'a Document, category: &[u8], name: &[u8]) -> Lookup<Entry<'a>> {
        let Some(dict) = self.dict else {
            return Lookup::Missing;
        };
        let Ok(cat) = dict.get(category) else {
            return Lookup::Missing;
        };
        let (category_id, cat) = match resolve(doc, cat) {
            Some((id, Object::Dictionary(d))) => (id, d),
            Some((_, Object::Null)) => return Lookup::Missing,
            _ => return Lookup::Broken("resource category"),
        };
        let Ok(raw) = cat.get(name) else {
            return Lookup::Missing;
        };
        match resolve(doc, raw) {
            Some((id, value)) => Lookup::Found(Entry {
                id,
                value,
                category_id,
            }),
            None => Lookup::Broken("resource reference"),
        }
    }

    /// The font `name` with its cache key. A missing name is `Missing` (`MISSING_FONT`); a value
    /// that is not a font dictionary is `Missing` too; a dangling reference is `Broken`.
    pub fn font(&self, doc: &'a Document, name: &[u8]) -> Lookup<(FontKey, &'a Dictionary)> {
        match self.entry(doc, b"Font", name) {
            Lookup::Found(e) => match e.value {
                Object::Dictionary(d) => {
                    let key = match e.id {
                        Some(id) => FontKey::Indirect(id),
                        None => FontKey::direct(e.category_id.unwrap_or(self.owner), name),
                    };
                    Lookup::Found((key, d))
                }
                _ => Lookup::Missing,
            },
            Lookup::Missing => Lookup::Missing,
            Lookup::Broken(w) => Lookup::Broken(w),
        }
    }

    /// Every `(name, key, dict)` of the `/Font` category that resolves to a font dictionary, in
    /// dictionary order (dangling entries are skipped: they are only an error when used).
    pub fn all_fonts(&self, doc: &'a Document) -> Vec<(Vec<u8>, FontKey, &'a Dictionary)> {
        let Some(dict) = self.dict else {
            return Vec::new();
        };
        let Some((category_id, Object::Dictionary(cat))) =
            dict.get(b"Font").ok().and_then(|c| resolve(doc, c))
        else {
            return Vec::new();
        };
        cat.iter()
            .filter_map(|(name, raw)| match resolve(doc, raw)? {
                (id, Object::Dictionary(d)) => {
                    let key = match id {
                        Some(id) => FontKey::Indirect(id),
                        None => FontKey::direct(category_id.unwrap_or(self.owner), name),
                    };
                    Some((name.clone(), key, d))
                }
                _ => None,
            })
            .collect()
    }

    /// Bytes of every name in the `/Font` category (what `all_fonts` copies), without copying.
    pub fn font_name_bytes(&self, doc: &'a Document) -> usize {
        self.dict
            .and_then(|d| d.get(b"Font").ok())
            .and_then(|c| resolve(doc, c))
            .and_then(|(_, o)| o.as_dict().ok())
            .map_or(0, |cat| cat.iter().map(|(name, _)| name.len()).sum())
    }

    /// Number of entries in the `/Font` category (0 when absent or unreadable).
    pub fn font_count(&self, doc: &'a Document) -> usize {
        self.dict
            .and_then(|d| d.get(b"Font").ok())
            .and_then(|c| resolve(doc, c))
            .and_then(|(_, o)| o.as_dict().ok())
            .map_or(0, Dictionary::len)
    }
}

/// A name, number or dictionary value from a dictionary, resolved.
pub fn get<'a>(doc: &'a Document, dict: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    let (_, obj) = resolve(doc, dict.get(key).ok()?)?;
    (!matches!(obj, Object::Null)).then_some(obj)
}
