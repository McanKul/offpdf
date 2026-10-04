//! Optional content and the logical structure tree (SPEC §A.6, §B.10, D19, D33): whether an
//! `/OC` group is visible in the default configuration, whether a marked-content id's structure
//! element (or one of its ancestors) carries `/ActualText`, and the structure order of a page's
//! MCIDs for reading order. Every traversal is bounded and cycle-checked.

use crate::pdf_engine::text_edit::context::{get, resolve};
use crate::pdf_engine::text_edit::limits::{
    NUMBER_TREE_NODES_MAX, STRUCT_CHAIN_MAX, STRUCT_ORDER_NODES_MAX,
};
use crate::pdf_engine::text_edit::reasons::TextReason;
use crate::pdf_engine::text_edit::state::OcState;
use lopdf::{Dictionary, Document, Object, ObjectId};
use std::collections::HashSet;

/// Nesting depth of the structure-order DFS.
const STRUCT_ORDER_DEPTH_MAX: usize = 64;

/// The default optional-content configuration (`/OCProperties /OCGs` and `/D`), read once per
/// snapshot (`SnapshotContext::oc_config`) into sorted id lists, so the state of a group costs
/// O(log n) however many groups the catalog lists or a page opens.
pub struct OcConfig {
    /// `None` when there is no readable configuration (every group is then `Unknown`).
    lists: Option<OcLists>,
}

struct OcLists {
    ocgs: Vec<ObjectId>,
    on: Vec<ObjectId>,
    off: Vec<ObjectId>,
    base_on: bool,
}

impl OcConfig {
    pub fn of(doc: &Document) -> OcConfig {
        OcConfig {
            lists: oc_lists(doc),
        }
    }

    /// The state of the optional-content group a `BDC /OC /Name` names, given the `/Properties`
    /// entry (normally a reference to the group). A single OCG listed in `/OCProperties /OCGs`
    /// is `Visible` when the default configuration `/D` has it on (BaseState ON and not in
    /// `/OFF`, or listed in `/ON`), `Hidden` when off; an OCMD, an inline dictionary, a group the
    /// catalog does not list, or anything unreadable is `Unknown` (all refused
    /// `OPTIONAL_CONTENT` but `Visible`).
    pub fn state(&self, doc: &Document, props: &Object) -> OcState {
        let Object::Reference(id) = props else {
            return OcState::Unknown;
        };
        let Some((_, Object::Dictionary(group))) = resolve(doc, props) else {
            return OcState::Unknown;
        };
        if !group.type_is(b"OCG") {
            return OcState::Unknown; // OCMD or not a group
        }
        let Some(lists) = &self.lists else {
            return OcState::Unknown;
        };
        let has = |ids: &[ObjectId]| ids.binary_search(id).is_ok();
        if !has(&lists.ocgs) {
            return OcState::Unknown;
        }
        let (on, off) = (has(&lists.on), has(&lists.off));
        if on && off {
            return OcState::Unknown;
        }
        if (lists.base_on && !off) || on {
            OcState::Visible
        } else {
            OcState::Hidden
        }
    }
}

/// The catalog's `/OCGs`, `/D /ON`, `/D /OFF` (direct references, sorted) and `/D /BaseState`;
/// `None` without `/OCProperties`, without a `/D` dictionary or with a BaseState other than
/// `/ON` or `/OFF` (`/Unchanged` has no meaning in `/D`).
fn oc_lists(doc: &Document) -> Option<OcLists> {
    let ocp = doc
        .catalog()
        .ok()
        .and_then(|c| get(doc, c, b"OCProperties"))
        .and_then(|o| o.as_dict().ok())?;
    let ids = |arr: Option<&Object>| -> Vec<ObjectId> {
        let mut ids: Vec<ObjectId> = arr
            .and_then(|a| a.as_array().ok())
            .map(|items| items.iter().filter_map(|i| i.as_reference().ok()).collect())
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    };
    let d = get(doc, ocp, b"D").and_then(|o| o.as_dict().ok())?;
    let base_on = match get(doc, d, b"BaseState") {
        None => true,
        Some(Object::Name(n)) if n.as_slice() == b"ON" => true,
        Some(Object::Name(n)) if n.as_slice() == b"OFF" => false,
        _ => return None,
    };
    Some(OcLists {
        ocgs: ids(get(doc, ocp, b"OCGs")),
        on: ids(get(doc, d, b"ON")),
        off: ids(get(doc, d, b"OFF")),
        base_on,
    })
}

/// The page's slice of the structure parent tree (`/StructParents` → `/ParentTree`), resolved
/// once per page and borrowed from the document (a parent array of millions of entries costs
/// nothing per model build, review T3 r3 LOW-2).
pub struct PageStruct<'a> {
    /// `Ok(None)`: the page has no structure parents (no MCID maps to an element).
    parents: Result<Option<&'a [Object]>, TextReason>,
}

impl<'a> PageStruct<'a> {
    pub fn of(doc: &'a Document, page_id: ObjectId) -> PageStruct<'a> {
        PageStruct {
            parents: page_parents(doc, page_id),
        }
    }

    /// Whether the structure element of `mcid` or one of its ancestors (≤ STRUCT_CHAIN_MAX) has
    /// `/ActualText`. A broken tree is `Err(ACTUAL_TEXT)` (the text might carry one: fail closed).
    pub fn actual_text(&self, doc: &Document, mcid: i64) -> Result<bool, TextReason> {
        let parents = match &self.parents {
            Ok(None) => return Ok(false),
            Ok(Some(p)) => p,
            Err(r) => return Err(*r),
        };
        let Some(elem) = usize::try_from(mcid).ok().and_then(|i| parents.get(i)) else {
            return Ok(false);
        };
        let mut cur = match resolve(doc, elem) {
            Some((_, Object::Dictionary(d))) => d,
            Some((_, Object::Null)) => return Ok(false),
            _ => return Err(TextReason::ActualText),
        };
        for _ in 0..STRUCT_CHAIN_MAX {
            if cur.has(b"ActualText") {
                return Ok(true);
            }
            if cur.type_is(b"StructTreeRoot") {
                return Ok(false);
            }
            match cur.get(b"P").ok().map(|p| resolve(doc, p)) {
                None => return Ok(false),
                Some(Some((_, Object::Dictionary(d)))) => cur = d,
                Some(_) => return Err(TextReason::ActualText),
            }
        }
        // A chain longer than the bound: treat as unknown.
        Err(TextReason::ActualText)
    }
}

/// `/StructParents` → `/ParentTree` (number tree, bounded) → the page's array.
fn page_parents(doc: &Document, page_id: ObjectId) -> Result<Option<&[Object]>, TextReason> {
    let bad = TextReason::ActualText;
    let Some(Object::Dictionary(page)) = doc.objects.get(&page_id) else {
        return Ok(None);
    };
    let key = match get(doc, page, b"StructParents") {
        None => return Ok(None),
        Some(Object::Integer(k)) => *k,
        Some(_) => return Err(bad),
    };
    let Some(root) = doc
        .catalog()
        .ok()
        .and_then(|c| get(doc, c, b"StructTreeRoot"))
    else {
        return Ok(None);
    };
    let root = root.as_dict().map_err(|_| bad)?;
    let Some(tree) = get(doc, root, b"ParentTree") else {
        return Ok(None);
    };
    match number_tree_get(doc, tree, key)? {
        None => Ok(None),
        Some(Object::Array(items)) => Ok(Some(items.as_slice())),
        Some(_) => Ok(None), // an object reference entry (OBJR parent), not marked content
    }
}

/// Looks `key` up in a number tree (bounded DFS, visited set; `/Limits` are not trusted).
fn number_tree_get<'a>(
    doc: &'a Document,
    tree: &'a Object,
    key: i64,
) -> Result<Option<&'a Object>, TextReason> {
    let bad = TextReason::ActualText;
    let mut stack = vec![tree];
    let mut seen: HashSet<ObjectId> = HashSet::new();
    let mut nodes = 0usize;
    while let Some(raw) = stack.pop() {
        nodes += 1;
        if nodes > NUMBER_TREE_NODES_MAX {
            return Err(bad);
        }
        if let Object::Reference(id) = raw {
            if !seen.insert(*id) {
                return Err(bad);
            }
        }
        let node = match resolve(doc, raw) {
            Some((_, Object::Dictionary(d))) => d,
            _ => return Err(bad),
        };
        if let Some(nums) = get(doc, node, b"Nums") {
            let items = nums.as_array().map_err(|_| bad)?;
            for pair in items.chunks(2) {
                if let [k, v] = pair {
                    if resolve(doc, k).map(|(_, o)| o.as_i64().ok()) == Some(Some(key)) {
                        return Ok(resolve(doc, v).map(|(_, o)| o));
                    }
                }
            }
        }
        if let Some(kids) = get(doc, node, b"Kids") {
            let items = kids.as_array().map_err(|_| bad)?;
            stack.extend(items.iter().rev());
        }
    }
    Ok(None)
}

/// The MCIDs of `page_id` in structure order (D33): a bounded DFS of `/StructTreeRoot /K`
/// (`/K` arrays in order, ≤ STRUCT_ORDER_NODES_MAX nodes, depth ≤ 64, visited set; `/Pg`
/// inherited down the tree; integer `/K` and `/Type /MCR` dicts with `/MCID` and optional `/Pg`).
/// `None` when the catalog has no structure tree or any budget, cycle or unresolvable reference
/// is met (the caller then uses XY-cut for the whole page).
pub fn structure_mcids(doc: &Document, page_id: ObjectId) -> Option<Vec<i64>> {
    let root = doc
        .catalog()
        .ok()
        .and_then(|c| get(doc, c, b"StructTreeRoot"))?
        .as_dict()
        .ok()?;
    let mut out = Vec::new();
    let mut seen_mcid = HashSet::new();
    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut nodes = 0usize;
    // (node, inherited /Pg, depth)
    let mut stack: Vec<(&Object, Option<ObjectId>, usize)> = Vec::new();
    let top = root.get(b"K").ok()?;
    stack.push((top, None, 0));
    while let Some((raw, pg, depth)) = stack.pop() {
        nodes += 1;
        if nodes > STRUCT_ORDER_NODES_MAX || depth > STRUCT_ORDER_DEPTH_MAX {
            return None;
        }
        if let Object::Reference(id) = raw {
            if !visited.insert(*id) {
                return None;
            }
        }
        let (_, node) = resolve(doc, raw)?;
        match node {
            Object::Integer(mcid) => {
                if pg == Some(page_id) && seen_mcid.insert(*mcid) {
                    out.push(*mcid);
                }
            }
            Object::Array(items) => {
                for item in items.iter().rev() {
                    stack.push((item, pg, depth + 1));
                }
            }
            Object::Dictionary(d) => {
                push_struct_dict(d, pg, depth, page_id, &mut stack, &mut out, &mut seen_mcid)?
            }
            Object::Null => {}
            _ => return None,
        }
    }
    Some(out)
}

fn push_struct_dict<'a>(
    d: &'a Dictionary,
    pg: Option<ObjectId>,
    depth: usize,
    page_id: ObjectId,
    stack: &mut Vec<(&'a Object, Option<ObjectId>, usize)>,
    out: &mut Vec<i64>,
    seen_mcid: &mut HashSet<i64>,
) -> Option<()> {
    let own_pg = match d.get(b"Pg").ok() {
        None => pg,
        Some(Object::Reference(id)) => Some(*id),
        Some(_) => return None,
    };
    if d.type_is(b"OBJR") {
        return Some(());
    }
    if d.type_is(b"MCR") {
        if let Ok(Object::Integer(mcid)) = d.get(b"MCID") {
            if own_pg == Some(page_id) && seen_mcid.insert(*mcid) {
                out.push(*mcid);
            }
        }
        return Some(());
    }
    if let Ok(k) = d.get(b"K") {
        stack.push((k, own_pg, depth + 1));
    }
    Some(())
}

/// §B.10 `struct_actual_text`: one lookup (production walks many MCIDs through `PageStruct`).
#[cfg(test)]
pub fn struct_actual_text(
    doc: &Document,
    page_id: ObjectId,
    mcid: i64,
) -> Result<bool, TextReason> {
    PageStruct::of(doc, page_id).actual_text(doc, mcid)
}
