//! Logical structure and page references (§E.2 rev. 2 `tagged_bookmarked(page)`): a structure
//! tree whose elements point at the page (`/Pg`), an annotation (`/P`), an outline item and a
//! link (`/Dest`), a named destination and `/OpenAction` — the references Word's tagged output
//! carries, none of which may make the page's content "shared" (F1).

use super::{DocBuilder, PageSpec, HELVETICA};

/// A structure tree with one `/P` element per MCID of `page`, listed in `order` (the structure
/// order); the MCIDs in `actual` carry `/ActualText`. Adds `/StructTreeRoot` and `/MarkInfo` to
/// the catalog and returns the page entry to add (`/StructParents 0`).
pub fn structure(d: &mut DocBuilder, page: u32, order: &[i64], actual: &[i64]) -> String {
    let root = d.b.alloc();
    let document = d.b.alloc();
    let mut by_mcid: Vec<(i64, u32)> = Vec::new();
    for mcid in order {
        let alt = if actual.contains(mcid) {
            format!(" /ActualText (alt {mcid})")
        } else {
            String::new()
        };
        let elem = d.b.add(format!(
            "<< /Type /StructElem /S /P /P {document} 0 R /Pg {page} 0 R /K {mcid}{alt} >>"
        ));
        by_mcid.push((*mcid, elem));
    }
    let kids = by_mcid
        .iter()
        .map(|(_, e)| format!("{e} 0 R"))
        .collect::<Vec<_>>()
        .join(" ");
    d.b.set(
        document,
        format!("<< /Type /StructElem /S /Document /P {root} 0 R /K [{kids}] >>"),
    );
    let max = order.iter().copied().max().unwrap_or(-1);
    let parents: Vec<String> = (0..=max)
        .map(|m| {
            by_mcid
                .iter()
                .find(|(x, _)| *x == m)
                .map_or("null".to_string(), |(_, e)| format!("{e} 0 R"))
        })
        .collect();
    d.b.set(
        root,
        format!(
            "<< /Type /StructTreeRoot /K {document} 0 R /ParentTree << /Nums [0 [{}]] >> \
             /ParentTreeNextKey 1 >>",
            parents.join(" ")
        ),
    );
    d.catalog_extra.push_str(&format!(
        " /StructTreeRoot {root} 0 R /MarkInfo << /Marked true >>"
    ));
    "/StructParents 0".to_string()
}

/// The structure tree of `mcids` plus a Link annotation (`/P`, `/Dest`), an outline item, a named
/// destination and `/OpenAction`, all pointing at `page`. Returns the page entries to add.
pub fn tagged_bookmarked(d: &mut DocBuilder, page: u32, mcids: &[i64]) -> String {
    let extra = structure(d, page, mcids, &[]);
    let link = d.b.add(format!(
        "<< /Type /Annot /Subtype /Link /Rect [72 600 200 620] /Border [0 0 0] /P {page} 0 R \
         /Dest [{page} 0 R /XYZ 0 792 0] >>"
    ));
    let outlines = d.b.alloc();
    let item = d.b.add(format!(
        "<< /Title (Start) /Parent {outlines} 0 R /Dest [{page} 0 R /Fit] >>"
    ));
    d.b.set(
        outlines,
        format!("<< /Type /Outlines /First {item} 0 R /Last {item} 0 R /Count 1 >>"),
    );
    d.catalog_extra.push_str(&format!(
        " /Outlines {outlines} 0 R /Names << /Dests << /Names [(start) [{page} 0 R /Fit]] >> >> \
         /OpenAction [{page} 0 R /Fit] /PageMode /UseOutlines"
    ));
    format!("{extra} /Annots [{link} 0 R] /Tabs /S")
}

/// A tagged, bookmarked Helvetica page (the helper on its own).
pub fn tagged_bookmarked_page() -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    let page = d.reserve();
    let extra = tagged_bookmarked(&mut d, page, &[0]);
    let content = b"/P <</MCID 0>> BDC BT /F1 12 Tf 72 700 Td (Tagged line) Tj ET EMC";
    d.page_at(
        page,
        PageSpec::new(content, &format!("/Font << /F1 {f} 0 R >>")).with(&extra),
    );
    d.build()
}
