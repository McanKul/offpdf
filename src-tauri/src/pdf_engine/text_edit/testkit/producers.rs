//! Producer-shaped fixtures (SPEC §E.2, T3): synthetic PDFs built in memory with `PdfBuilder`
//! and the T2 font builders, shaped like what Word, LibreOffice, Chrome/Skia, Quartz, pdfTeX,
//! XeTeX, InDesign, report generators, scanners and imposition tools write (§A.9), plus the
//! geometry, syntax and file-level edges the walker, the runs and the classifier must handle.
//! Nothing here is committed (§E.1); every builder has a smoke test in
//! `tests_walk/producers.rs`.

mod edges;
mod files;
mod office;
mod tagging;

pub use edges::*;
pub use files::*;
pub use office::*;
pub use tagging::*;

use super::pdf::{PdfBuilder, XrefStyle};

/// Standard-14 Helvetica with WinAnsi: the font of the edge fixtures (`/F1`).
pub const HELVETICA: &str =
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>";

/// One page: its content parts, the inside of its `/Resources` dictionary, its `/MediaBox`
/// (`None` = none on the page) and extra page entries.
#[derive(Debug, Clone)]
pub struct PageSpec {
    pub parts: Vec<Vec<u8>>,
    pub resources: String,
    pub media: Option<String>,
    pub extra: String,
}

impl PageSpec {
    pub fn new(content: &[u8], resources: &str) -> PageSpec {
        PageSpec {
            parts: vec![content.to_vec()],
            resources: resources.to_string(),
            media: Some("[0 0 612 792]".to_string()),
            extra: String::new(),
        }
    }

    pub fn parts(parts: &[&[u8]], resources: &str) -> PageSpec {
        PageSpec {
            parts: parts.iter().map(|p| p.to_vec()).collect(),
            ..PageSpec::new(b"", resources)
        }
    }

    /// Adds page dictionary entries, e.g. `/Rotate 90`.
    pub fn with(mut self, extra: &str) -> PageSpec {
        self.extra.push(' ');
        self.extra.push_str(extra);
        self
    }

    pub fn media(mut self, media: Option<&str>) -> PageSpec {
        self.media = media.map(str::to_string);
        self
    }
}

/// A document under construction: catalog and `/Pages` ids are reserved up front, so fixtures
/// can reference pages from other objects (structure trees, links) before they are written.
#[derive(Debug, Clone)]
pub struct DocBuilder {
    pub b: PdfBuilder,
    pub catalog: u32,
    pub pages: u32,
    pub page_ids: Vec<u32>,
    pub content_ids: Vec<Vec<u32>>,
    /// The `/Kids` of the root `/Pages`, in order.
    pub kids: Vec<u32>,
    pub catalog_extra: String,
    pub pages_extra: String,
}

impl Default for DocBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DocBuilder {
    pub fn new() -> DocBuilder {
        let mut b = PdfBuilder::new();
        let catalog = b.alloc();
        let pages = b.alloc();
        DocBuilder {
            b,
            catalog,
            pages,
            page_ids: Vec::new(),
            content_ids: Vec::new(),
            kids: Vec::new(),
            catalog_extra: String::new(),
            pages_extra: String::new(),
        }
    }

    pub fn add(&mut self, body: impl AsRef<[u8]>) -> u32 {
        self.b.add(body)
    }

    /// An object id for a page written later with `page_at`.
    pub fn reserve(&mut self) -> u32 {
        self.b.alloc()
    }

    /// Adds a page (its parts as unfiltered streams); returns its id.
    pub fn page(&mut self, spec: PageSpec) -> u32 {
        let id = self.reserve();
        self.page_at(id, spec)
    }

    /// Writes a page at a reserved id.
    pub fn page_at(&mut self, id: u32, spec: PageSpec) -> u32 {
        let ids: Vec<u32> = spec
            .parts
            .iter()
            .map(|p| self.b.add_stream("", p))
            .collect();
        let contents = refs(&ids);
        self.content_ids.push(ids);
        self.page_raw_at(id, &contents, &spec)
    }

    /// A page whose `/Contents` is written as given (shared streams, filtered parts).
    pub fn page_raw(&mut self, contents: &str, spec: &PageSpec) -> u32 {
        let id = self.reserve();
        self.content_ids.push(Vec::new());
        self.page_raw_at(id, contents, spec)
    }

    fn page_raw_at(&mut self, id: u32, contents: &str, spec: &PageSpec) -> u32 {
        let media = spec
            .media
            .as_ref()
            .map_or(String::new(), |m| format!("/MediaBox {m}"));
        self.b.set(
            id,
            format!(
                "<< /Type /Page /Parent {} 0 R {media} /Contents {contents} /Resources << {} >> {} >>",
                self.pages, spec.resources, spec.extra
            ),
        );
        self.page_ids.push(id);
        self.kids.push(id);
        id
    }

    pub fn trailer(&self) -> String {
        format!("/Root {} 0 R", self.catalog)
    }

    fn finish(&mut self) {
        let kids = self
            .kids
            .iter()
            .map(|k| format!("{k} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        self.b.set(
            self.pages,
            format!(
                "<< /Type /Pages /Kids [{kids}] /Count {} {} >>",
                self.kids.len(),
                self.pages_extra
            ),
        );
        self.b.set(
            self.catalog,
            format!(
                "<< /Type /Catalog /Pages {} 0 R {} >>",
                self.pages, self.catalog_extra
            ),
        );
    }

    pub fn build(mut self) -> Vec<u8> {
        self.finish();
        self.b.build(&self.trailer())
    }

    pub fn build_with(mut self, style: &XrefStyle) -> Vec<u8> {
        self.finish();
        self.b.build_with(&self.trailer(), style)
    }
}

/// `N 0 R` for one id, `[a 0 R b 0 R …]` for several.
pub fn refs(ids: &[u32]) -> String {
    match ids {
        [one] => format!("{one} 0 R"),
        many => format!(
            "[{}]",
            many.iter()
                .map(|i| format!("{i} 0 R"))
                .collect::<Vec<_>>()
                .join(" ")
        ),
    }
}

/// A one-page document drawing `content` with Helvetica as `/F1`, plus extra resources and page
/// entries.
pub fn helvetica_doc(content: &[u8], extra_resources: &str, page_extra: &str) -> Vec<u8> {
    let mut d = DocBuilder::new();
    let f = d.add(HELVETICA);
    d.page(
        PageSpec::new(
            content,
            &format!("/Font << /F1 {f} 0 R >> {extra_resources}"),
        )
        .with(page_extra),
    );
    d.build()
}

/// `helvetica_doc` without extras.
pub fn helvetica_page(content: &[u8]) -> Vec<u8> {
    helvetica_doc(content, "", "")
}
