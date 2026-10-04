//! Fixture PDF builder (T1): raw objects with exact control over bytes, written with a classic
//! xref table, an xref stream (optionally with an object stream) or as a hybrid-reference file.
//! Fixtures are generated in temp by test code only (§E.1).

use std::collections::BTreeMap;
use std::io::Write;

/// Zlib-wrapped Flate of `data`.
pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).expect("zlib write");
    e.finish().expect("zlib finish")
}

/// A zlib stream that inflates to `mib` MiB of zero bytes, built from one sync-flushed 1 MiB
/// chunk repeated (≈1 KiB of input per MiB of output). Its Adler-32 is wrong on purpose.
pub fn zlib_zero_bomb(mib: usize) -> Vec<u8> {
    let zeros = vec![0u8; 1 << 20];
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    e.write_all(&zeros).expect("bomb write");
    e.flush().expect("bomb flush");
    let first = e.get_ref().len();
    e.write_all(&zeros).expect("bomb write");
    e.flush().expect("bomb flush");
    let second = e.get_ref().len();
    let full = e.finish().expect("bomb finish");
    let chunk = full[first..second].to_vec();
    let mut out = full[..second].to_vec();
    for _ in 2..mib.max(2) {
        out.extend_from_slice(&chunk);
    }
    out.extend_from_slice(&full[second..]);
    out
}

/// How the cross-reference data is written.
#[derive(Debug, Clone)]
pub enum XrefStyle {
    /// Classic `xref` table.
    Table,
    /// An xref stream; `in_objstm` objects go into one object stream. `bomb_mib` replaces the
    /// xref stream data with a Flate bomb of that many MiB.
    Stream {
        in_objstm: Vec<u32>,
        bomb_mib: Option<usize>,
    },
    /// Classic table plus `/XRefStm` in the same (newest) trailer, no `/Prev`. `in_objstm`
    /// objects are listed only in the xref stream; the object stream itself is also listed in the
    /// classic table when `objstm_in_classic` (Word style).
    Hybrid {
        in_objstm: Vec<u32>,
        objstm_in_classic: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct PdfBuilder {
    objects: BTreeMap<u32, Vec<u8>>,
    next: u32,
}

impl PdfBuilder {
    pub fn new() -> Self {
        PdfBuilder {
            objects: BTreeMap::new(),
            next: 1,
        }
    }

    pub fn alloc(&mut self) -> u32 {
        let id = self.next;
        self.next += 1;
        id
    }

    pub fn set(&mut self, id: u32, body: impl AsRef<[u8]>) {
        self.next = self.next.max(id + 1);
        self.objects.insert(id, body.as_ref().to_vec());
    }

    pub fn add(&mut self, body: impl AsRef<[u8]>) -> u32 {
        let id = self.alloc();
        self.set(id, body);
        id
    }

    pub fn stream_body(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut body = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        body.extend_from_slice(data);
        body.extend_from_slice(b"\nendstream");
        body
    }

    pub fn set_stream(&mut self, id: u32, dict: &str, data: &[u8]) {
        self.set(id, Self::stream_body(dict, data));
    }

    pub fn add_stream(&mut self, dict: &str, data: &[u8]) -> u32 {
        self.add(Self::stream_body(dict, data))
    }

    pub fn add_flate(&mut self, dict: &str, data: &[u8]) -> u32 {
        self.add_stream(&format!("/Filter /FlateDecode {dict}"), &zlib(data))
    }

    pub fn body(&self, id: u32) -> Option<&[u8]> {
        self.objects.get(&id).map(Vec::as_slice)
    }

    fn header() -> Vec<u8> {
        b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec()
    }

    fn write_obj(out: &mut Vec<u8>, id: u32, body: &[u8]) -> usize {
        let off = out.len();
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
        off
    }

    fn table(entries: &BTreeMap<u32, usize>) -> Vec<u8> {
        // subsections of consecutive ids, plus the free head entry 0
        let mut ids: Vec<u32> = entries.keys().copied().collect();
        ids.insert(0, 0);
        let mut out = b"xref\n".to_vec();
        let mut i = 0;
        while i < ids.len() {
            let mut j = i;
            while j + 1 < ids.len() && ids[j + 1] == ids[j] + 1 {
                j += 1;
            }
            out.extend_from_slice(format!("{} {}\n", ids[i], j - i + 1).as_bytes());
            for id in &ids[i..=j] {
                match entries.get(id) {
                    Some(off) if *id != 0 => {
                        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes())
                    }
                    _ => out.extend_from_slice(b"0000000000 65535 f \n"),
                }
            }
            i = j + 1;
        }
        out
    }

    /// Writes the file. `trailer` holds extra trailer entries, e.g. `/Root 1 0 R`.
    pub fn build(&self, trailer: &str) -> Vec<u8> {
        self.build_with(trailer, &XrefStyle::Table)
    }

    pub fn build_with(&self, trailer: &str, style: &XrefStyle) -> Vec<u8> {
        let mut out = Self::header();
        let max = self.objects.keys().max().copied().unwrap_or(0);
        match style {
            XrefStyle::Table => {
                let mut offs = BTreeMap::new();
                for (id, body) in &self.objects {
                    offs.insert(*id, Self::write_obj(&mut out, *id, body));
                }
                let x = out.len();
                out.extend_from_slice(&Self::table(&offs));
                out.extend_from_slice(
                    format!(
                        "trailer\n<< /Size {} {trailer} >>\nstartxref\n{x}\n%%EOF\n",
                        max + 1
                    )
                    .as_bytes(),
                );
            }
            XrefStyle::Stream {
                in_objstm,
                bomb_mib,
            } => {
                let (offs, objstm) = self.write_with_objstm(&mut out, in_objstm, max + 1);
                let xref_id = max + 2;
                let x = out.len();
                let data = Self::xref_rows(
                    &offs,
                    in_objstm,
                    objstm.map(|_| max + 1),
                    xref_id,
                    x,
                    max + 3,
                );
                let (dict, payload) = match bomb_mib {
                    Some(mib) => ("/Filter /FlateDecode".to_string(), zlib_zero_bomb(*mib)),
                    None => (String::new(), data),
                };
                let body = Self::stream_body(
                    &format!("/Type /XRef /Size {} /W [1 4 2] {dict} {trailer}", max + 3),
                    &payload,
                );
                Self::write_obj(&mut out, xref_id, &body);
                out.extend_from_slice(format!("startxref\n{x}\n%%EOF\n").as_bytes());
            }
            XrefStyle::Hybrid {
                in_objstm,
                objstm_in_classic,
            } => {
                let (offs, objstm) = self.write_with_objstm(&mut out, in_objstm, max + 1);
                let xref_id = max + 2;
                let xs = out.len();
                let data = Self::xref_rows(
                    &offs,
                    in_objstm,
                    objstm.map(|_| max + 1),
                    xref_id,
                    xs,
                    max + 3,
                );
                let body =
                    Self::stream_body(&format!("/Type /XRef /Size {} /W [1 4 2]", max + 3), &data);
                Self::write_obj(&mut out, xref_id, &body);
                let mut classic: BTreeMap<u32, usize> = offs
                    .iter()
                    .filter(|(id, _)| !in_objstm.contains(id))
                    .map(|(a, b)| (*a, *b))
                    .collect();
                if !objstm_in_classic {
                    if let Some(id) = objstm.map(|_| max + 1) {
                        classic.remove(&id);
                    }
                }
                let x = out.len();
                out.extend_from_slice(&Self::table(&classic));
                out.extend_from_slice(
                    format!(
                        "trailer\n<< /Size {} {trailer} /XRefStm {xs} >>\nstartxref\n{x}\n%%EOF\n",
                        max + 3
                    )
                    .as_bytes(),
                );
            }
        }
        out
    }

    /// Writes every object not in `in_objstm`, then (if any) one object stream `objstm_id`
    /// holding the others. Returns the offsets of top-level objects (incl. the object stream).
    fn write_with_objstm(
        &self,
        out: &mut Vec<u8>,
        in_objstm: &[u32],
        objstm_id: u32,
    ) -> (BTreeMap<u32, usize>, Option<u32>) {
        let mut offs = BTreeMap::new();
        for (id, body) in &self.objects {
            if !in_objstm.contains(id) {
                offs.insert(*id, Self::write_obj(out, *id, body));
            }
        }
        if in_objstm.is_empty() {
            return (offs, None);
        }
        let mut header = String::new();
        let mut objs = Vec::new();
        for id in in_objstm {
            header.push_str(&format!("{id} {} ", objs.len()));
            objs.extend_from_slice(self.objects.get(id).expect("objstm member"));
            objs.push(b'\n');
        }
        let mut data = header.clone().into_bytes();
        data.extend_from_slice(&objs);
        let body = Self::stream_body(
            &format!(
                "/Type /ObjStm /N {} /First {} /Filter /FlateDecode",
                in_objstm.len(),
                header.len()
            ),
            &zlib(&data),
        );
        offs.insert(objstm_id, Self::write_obj(out, objstm_id, &body));
        (offs, Some(objstm_id))
    }

    fn xref_rows(
        offs: &BTreeMap<u32, usize>,
        in_objstm: &[u32],
        objstm: Option<u32>,
        xref_id: u32,
        xref_off: usize,
        size: u32,
    ) -> Vec<u8> {
        let mut data = Vec::new();
        for id in 0..size {
            let (t, f2, f3): (u8, u32, u16) = if id == xref_id {
                (1, xref_off as u32, 0)
            } else if let Some(pos) = in_objstm.iter().position(|x| *x == id) {
                (2, objstm.expect("object stream id"), pos as u16)
            } else if let Some(off) = offs.get(&id) {
                (1, *off as u32, 0)
            } else {
                (0, 0, if id == 0 { 65535 } else { 0 })
            };
            data.push(t);
            data.extend_from_slice(&f2.to_be_bytes());
            data.extend_from_slice(&f3.to_be_bytes());
        }
        data
    }
}

/// A document with a shared Helvetica font on the `/Pages` node. Each page lists its content
/// parts; one part is written as `/Contents N 0 R`, several as a direct array.
pub struct Doc {
    pub b: PdfBuilder,
    pub catalog: u32,
    pub pages: u32,
    pub page_ids: Vec<u32>,
    pub content_ids: Vec<Vec<u32>>,
}

impl Doc {
    pub fn new(pages: &[&[&[u8]]]) -> Doc {
        let mut b = PdfBuilder::new();
        let catalog = b.alloc();
        let pages_id = b.alloc();
        let font = b.add(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
        );
        let mut page_ids = Vec::new();
        let mut content_ids = Vec::new();
        for parts in pages {
            let ids: Vec<u32> = parts.iter().map(|p| b.add_stream("", p)).collect();
            let contents = match ids.as_slice() {
                [one] => format!("{one} 0 R"),
                many => format!(
                    "[{}]",
                    many.iter()
                        .map(|i| format!("{i} 0 R"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            };
            page_ids.push(b.add(format!(
                "<< /Type /Page /Parent {pages_id} 0 R /Contents {contents} >>"
            )));
            content_ids.push(ids);
        }
        let kids = page_ids
            .iter()
            .map(|i| format!("{i} 0 R"))
            .collect::<Vec<_>>()
            .join(" ");
        b.set(
            pages_id,
            format!(
                "<< /Type /Pages /Kids [{kids}] /Count {} /MediaBox [0 0 612 792] /Resources << /Font << /F1 {font} 0 R >> >> >>",
                page_ids.len()
            ),
        );
        b.set(
            catalog,
            format!("<< /Type /Catalog /Pages {pages_id} 0 R >>"),
        );
        Doc {
            b,
            catalog,
            pages: pages_id,
            page_ids,
            content_ids,
        }
    }

    pub fn trailer(&self) -> String {
        format!("/Root {} 0 R", self.catalog)
    }

    pub fn build(&self) -> Vec<u8> {
        self.b.build(&self.trailer())
    }

    pub fn build_with(&self, style: &XrefStyle) -> Vec<u8> {
        self.b.build_with(&self.trailer(), style)
    }
}

/// One page, one content stream.
pub fn simple_pdf(content: &[u8]) -> Vec<u8> {
    Doc::new(&[&[content]]).build()
}
