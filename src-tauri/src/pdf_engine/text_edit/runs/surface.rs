//! Typing surfaces of a page's runs (SPEC §B.11 `surface`, review T3 r3 HIGH-1). A surface is
//! built once per surface key — the ExtGState font, or the primary's resource name — and shared by
//! every run that has it: a page of 20,000 lines in one font holds one list of sibling names, not
//! 20,000 copies of it, and `typing_surface` (O(F²)) runs once per key, not once per run.
//!
//! A sibling group never holds a font whose resource name is longer than
//! `SIBLING_NAME_BYTES_MAX` (ISO 32000 Annex C): such a font types only its own runs. Every
//! surface, its names and the scratch that builds it are charged to the page-model budget.

use super::Unit;
use crate::pdf_engine::text_edit::fonts::{typing_surface, FontKey, FontModel, TypingSurface};
use crate::pdf_engine::text_edit::limits::SIBLING_NAME_BYTES_MAX;
use crate::pdf_engine::text_edit::state::Bytes;
use crate::pdf_engine::text_edit::walker::budget::{arc_slice, map_entry, ModelBudget};
use crate::pdf_engine::text_edit::walker::{PageWalk, ShowRecord, Stop};
use std::borrow::Cow;
use std::collections::HashMap;
use std::mem::size_of;
use std::sync::Arc;

/// What a surface depends on: the ExtGState font's key, or the primary's font resource name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum SurfaceKey {
    ExtGState(Option<FontKey>),
    Resource(Option<Bytes>),
}

pub(super) fn surface_key(primary: &ShowRecord) -> SurfaceKey {
    match primary.before.text.font.as_ref() {
        Some(f) if f.from_extgstate => SurfaceKey::ExtGState(primary.font_key),
        f => SurfaceKey::Resource(f.and_then(|f| f.resource.clone())),
    }
}

type PageFonts = [(Vec<u8>, Arc<FontModel>)];

/// The page fonts a sibling group may hold: every one whose name is at most
/// `SIBLING_NAME_BYTES_MAX` bytes (borrowed when that is all of them).
fn sibling_fonts(walk: &PageWalk) -> Cow<'_, PageFonts> {
    let fonts = walk.page_fonts.as_slice();
    if fonts.iter().all(|(n, _)| n.len() <= SIBLING_NAME_BYTES_MAX) {
        return Cow::Borrowed(fonts);
    }
    Cow::Owned(
        fonts
            .iter()
            .filter(|(n, _)| n.len() <= SIBLING_NAME_BYTES_MAX)
            .cloned()
            .collect(),
    )
}

/// The surface of `primary` over `siblings`: its ExtGState font alone (§A.1.1), its resource's
/// sibling group, or — for a name too long to join one — that font alone.
fn surface_over(walk: &PageWalk, siblings: &PageFonts, primary: &ShowRecord) -> TypingSurface {
    let font = primary.before.text.font.as_ref();
    if font.is_some_and(|f| f.from_extgstate) {
        return TypingSurface {
            fonts: primary
                .font
                .iter()
                .map(|m| (Vec::new(), Arc::clone(m)))
                .collect(),
        };
    }
    match font.and_then(|f| f.resource.as_deref()) {
        Some(res) if res.len() > SIBLING_NAME_BYTES_MAX => TypingSurface {
            fonts: walk
                .page_fonts
                .iter()
                .find(|(n, _)| n.as_slice() == res)
                .map(|(n, m)| (n.clone(), Arc::clone(m)))
                .into_iter()
                .collect(),
        },
        Some(res) => typing_surface(siblings, res),
        None => TypingSurface { fonts: Vec::new() },
    }
}

/// The typing surface of a run (`PageModel::surface`): its ExtGState font alone, or the
/// primary's sibling group from the page fonts.
pub fn surface_of(walk: &PageWalk, primary: &ShowRecord) -> TypingSurface {
    surface_over(walk, &sibling_fonts(walk), primary)
}

/// One surface as runs use it.
pub(super) struct SurfaceInfo {
    /// Font resources, primary first (empty for an ExtGState font) — `TextRun::surface`.
    pub names: Arc<[Vec<u8>]>,
    pub has_space: bool,
    /// Some font of the surface can type something (else `NO_WRITABLE_GLYPHS`).
    pub writable: bool,
    /// Sibling name → content hash (joins, §A.1.1).
    members: HashMap<Vec<u8>, u64>,
}

impl SurfaceInfo {
    /// No surface: a line that is never typed in (Form text, §A.6 `NESTED_FORM`).
    pub(super) fn none() -> SurfaceInfo {
        SurfaceInfo {
            names: Arc::from(Vec::new()),
            has_space: false,
            writable: false,
            members: HashMap::new(),
        }
    }
}

/// The surfaces of one page, built on first use and charged to the page-model budget.
pub(super) struct Surfaces<'w> {
    walk: &'w PageWalk,
    siblings: Cow<'w, PageFonts>,
    names_bytes: usize,
    by_key: HashMap<SurfaceKey, Arc<SurfaceInfo>>,
    /// Per font model (by address; the page fonts keep them alive): a non-empty alphabet.
    alphabets: HashMap<usize, bool>,
}

impl<'w> Surfaces<'w> {
    pub(super) fn new(walk: &'w PageWalk, mem: &mut ModelBudget) -> Result<Surfaces<'w>, Stop> {
        let all: usize = walk.page_fonts.iter().map(|(n, _)| n.len()).sum();
        mem.scratch(all.saturating_add(walk.page_fonts.len() * size_of::<(Vec<u8>, usize)>()))?;
        let siblings = sibling_fonts(walk);
        let names_bytes = siblings.iter().map(|(n, _)| n.len()).sum();
        Ok(Surfaces {
            walk,
            siblings,
            names_bytes,
            by_key: HashMap::new(),
            alphabets: HashMap::new(),
        })
    }

    /// The surface of the run whose primary is `primary`.
    pub(super) fn of(
        &mut self,
        primary: &ShowRecord,
        mem: &mut ModelBudget,
    ) -> Result<Arc<SurfaceInfo>, Stop> {
        let key = surface_key(primary);
        if let Some(info) = self.by_key.get(&key) {
            return Ok(Arc::clone(info));
        }
        // `typing_surface` copies every sibling's name (and a long primary's own).
        let own = primary
            .before
            .text
            .font
            .as_ref()
            .and_then(|f| f.resource.as_ref())
            .map_or(0, |r| r.len());
        let entry = size_of::<(Vec<u8>, Arc<FontModel>)>();
        let building = self
            .names_bytes
            .saturating_add(own)
            .saturating_add((self.siblings.len() + 1).saturating_mul(entry));
        mem.scratch(building)?;
        let surface = surface_over(self.walk, &self.siblings, primary);
        let info = self.info(&key, surface, mem)?;
        mem.unscratch(building);
        mem.scratch(map_entry::<SurfaceKey, Arc<SurfaceInfo>>())?;
        self.by_key.insert(key, Arc::clone(&info));
        Ok(info)
    }

    fn info(
        &mut self,
        key: &SurfaceKey,
        surface: TypingSurface,
        mem: &mut ModelBudget,
    ) -> Result<Arc<SurfaceInfo>, Stop> {
        let has_space = surface.has_space();
        let mut writable = false;
        for (_, m) in &surface.fonts {
            let at = Arc::as_ptr(m) as usize;
            let nonempty = match self.alphabets.get(&at) {
                Some(known) => *known,
                None => {
                    mem.scratch(map_entry::<usize, bool>())?;
                    let known = !m.alphabet().is_empty();
                    self.alphabets.insert(at, known);
                    known
                }
            };
            if nonempty {
                writable = true;
                break;
            }
        }
        let copies: usize = surface.fonts.iter().map(|(n, _)| n.len()).sum();
        let count = surface.fonts.len();
        mem.scratch(copies.saturating_add(count * map_entry::<Vec<u8>, u64>()))?;
        let members: HashMap<Vec<u8>, u64> = surface
            .fonts
            .iter()
            .map(|(n, m)| (n.clone(), m.content_hash))
            .collect();
        let names: Vec<Vec<u8>> = if matches!(key, SurfaceKey::ExtGState(_)) {
            Vec::new()
        } else {
            surface.fonts.into_iter().map(|(n, _)| n).collect()
        };
        let held: usize = names.iter().map(Vec::capacity).sum();
        mem.hold(held.saturating_add(arc_slice(names.len(), size_of::<Vec<u8>>())))?;
        let info = SurfaceInfo {
            names: names.into(),
            has_space,
            writable,
            members,
        };
        Ok(Arc::new(info))
    }

    /// Whether `y` (a resource with content hash `y_hash`) is in the sibling group of the resource
    /// `x` drawn by `rec` (§A.1.1 fonts join).
    pub(super) fn joins(
        &mut self,
        rec: &ShowRecord,
        y: &[u8],
        y_hash: u64,
        mem: &mut ModelBudget,
    ) -> Result<bool, Stop> {
        let info = self.of(rec, mem)?;
        Ok(info.members.get(y).is_some_and(|h| *h == y_hash))
    }
}

/// The bytes a glyph unit keeps beyond its slot: its text and its font resource name.
pub(super) fn unit_heap(u: &Unit) -> usize {
    match u {
        Unit::Glyph { font_res, text, .. } => {
            text.capacity() + font_res.as_ref().map_or(0, Vec::capacity)
        }
        Unit::Kern { .. } => 0,
    }
}
