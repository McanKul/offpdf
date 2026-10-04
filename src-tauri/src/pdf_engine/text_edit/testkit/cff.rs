//! CFF font program builder for tests (T2, SPEC §E.1): bare CFF programs (`FontFile3 /Type1C`,
//! `/CIDFontType0C`) with name-keyed or CID-keyed charsets, Standard / Expert / custom (format 0
//! or 1, with supplements) encodings, minimal Type2 outlines or raw charstrings, global and
//! local subroutines, and padded Top/Private DICTs.

/// The 391 CFF standard strings; standard glyph names must use these SIDs.
pub(crate) use crate::pdf_engine::text_edit::fonts::cff_layout::STANDARD_STRINGS;

#[derive(Debug, Clone)]
pub struct CffGlyph {
    pub name: String,
    pub outline: bool,
    pub cid: u16,
    /// A raw Type2 charstring used instead of the box (or the bare `endchar`).
    pub charstring: Option<Vec<u8>>,
}

/// The program's own encoding (Top DICT operator 16).
#[derive(Debug, Clone)]
pub enum CffEncodingSpec {
    Standard,
    Expert,
    /// Format 0: `codes[i]` → GID i+1.
    Format0(Vec<u8>),
    /// Format 1: ranges `(first, nLeft)` → consecutive GIDs from 1.
    Format1(Vec<(u8, u8)>),
}

#[derive(Debug, Clone)]
pub struct CffBuilder {
    pub font_name: String,
    /// GID 1, 2, … (GID 0 is a drawn `.notdef`).
    pub glyphs: Vec<CffGlyph>,
    pub encoding: CffEncodingSpec,
    /// Encoding supplements `(code, glyph name)` (format high bit).
    pub supplements: Vec<(u8, String)>,
    pub cid_keyed: bool,
    /// Global subroutines (Global Subr INDEX).
    pub gsubrs: Vec<Vec<u8>>,
    /// Local subroutines (`Subrs` of the Private DICT; of FD 0 for CID-keyed fonts).
    pub subrs: Vec<Vec<u8>>,
    /// Charset format: 0 (an array), or 1 / 2 with one range per glyph (the most ranges).
    pub charset_format: u8,
    /// CID-keyed fonts: FDSelect format 3 (one range) or 0 (one byte per glyph); all FD 0.
    pub fdselect_format: u8,
    /// Bytes appended to the Private DICT after its entries (numbers or operators the readers
    /// skip), and to the Top DICT.
    pub private_padding: Vec<u8>,
    pub top_padding: Vec<u8>,
}

impl CffBuilder {
    pub fn new(font_name: &str) -> Self {
        CffBuilder {
            font_name: font_name.to_string(),
            glyphs: Vec::new(),
            encoding: CffEncodingSpec::Standard,
            supplements: Vec::new(),
            cid_keyed: false,
            gsubrs: Vec::new(),
            subrs: Vec::new(),
            charset_format: 0,
            fdselect_format: 3,
            private_padding: Vec::new(),
            top_padding: Vec::new(),
        }
    }

    /// A CID-keyed glyph (next GID) for `cid` with a raw charstring.
    pub fn raw_cid_glyph(mut self, cid: u16, charstring: Vec<u8>) -> Self {
        self.cid_keyed = true;
        self.glyphs.push(CffGlyph {
            name: String::new(),
            outline: true,
            cid,
            charstring: Some(charstring),
        });
        self
    }

    /// A name-keyed glyph (next GID) with a raw charstring.
    pub fn raw_glyph(mut self, name: &str, charstring: Vec<u8>) -> Self {
        self.glyphs.push(CffGlyph {
            name: name.to_string(),
            outline: true,
            cid: 0,
            charstring: Some(charstring),
        });
        self
    }

    /// A name-keyed glyph (next GID).
    pub fn glyph(mut self, name: &str, outline: bool) -> Self {
        self.glyphs.push(CffGlyph {
            name: name.to_string(),
            outline,
            cid: 0,
            charstring: None,
        });
        self
    }

    /// A CID-keyed glyph (next GID) for `cid`.
    pub fn cid_glyph(mut self, cid: u16, outline: bool) -> Self {
        self.cid_keyed = true;
        self.glyphs.push(CffGlyph {
            name: String::new(),
            outline,
            cid,
            charstring: None,
        });
        self
    }

    pub fn encoding(mut self, encoding: CffEncodingSpec) -> Self {
        self.encoding = encoding;
        self
    }

    pub fn supplement(mut self, code: u8, name: &str) -> Self {
        self.supplements.push((code, name.to_string()));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut custom: Vec<String> = Vec::new();
        let mut custom_sids: std::collections::HashMap<String, u16> = Default::default();
        let mut sid = |name: &str, custom: &mut Vec<String>| -> u16 {
            if let Some(i) = STANDARD_STRINGS.iter().position(|s| *s == name) {
                return i as u16;
            }
            if let Some(sid) = custom_sids.get(name) {
                return *sid;
            }
            custom.push(name.to_string());
            let sid = (391 + custom.len() - 1) as u16;
            custom_sids.insert(name.to_string(), sid);
            sid
        };
        let (ros_registry, ros_ordering) = if self.cid_keyed {
            (sid("Adobe", &mut custom), sid("Identity", &mut custom))
        } else {
            (0, 0)
        };
        let mut charset = vec![self.charset_format];
        for g in &self.glyphs {
            let v = if self.cid_keyed {
                g.cid
            } else {
                sid(&g.name, &mut custom)
            };
            charset.extend_from_slice(&v.to_be_bytes());
            match self.charset_format {
                1 => charset.push(0),
                2 => charset.extend_from_slice(&0u16.to_be_bytes()),
                _ => {}
            }
        }
        let mut supplements = Vec::new();
        for (code, name) in &self.supplements {
            supplements.push(*code);
            supplements.extend_from_slice(&sid(name, &mut custom).to_be_bytes());
        }
        let sup_flag = if self.supplements.is_empty() { 0 } else { 0x80 };
        let encoding: Option<Vec<u8>> = match &self.encoding {
            CffEncodingSpec::Standard | CffEncodingSpec::Expert => None,
            CffEncodingSpec::Format0(codes) => {
                let mut e = vec![sup_flag, codes.len() as u8];
                e.extend_from_slice(codes);
                Some(e)
            }
            CffEncodingSpec::Format1(ranges) => {
                let mut e = vec![1 | sup_flag, ranges.len() as u8];
                for (first, left) in ranges {
                    e.extend_from_slice(&[*first, *left]);
                }
                Some(e)
            }
        };
        let encoding = encoding.map(|mut e| {
            if !self.supplements.is_empty() {
                e.push(self.supplements.len() as u8);
                e.extend_from_slice(&supplements);
            }
            e
        });
        let mut charstrings = vec![box_charstring()];
        for g in &self.glyphs {
            charstrings.push(match (&g.charstring, g.outline) {
                (Some(raw), _) => raw.clone(),
                (None, true) => box_charstring(),
                (None, false) => vec![14],
            });
        }
        let charstrings = index(&charstrings);
        let mut private = vec![139, 20, 139, 21]; // defaultWidthX 0, nominalWidthX 0
        let subrs = if self.subrs.is_empty() {
            Vec::new()
        } else {
            // Subrs (op 19), relative to the Private DICT: right after it (10 bytes + padding).
            private.extend(int((10 + self.private_padding.len()) as i32));
            private.push(19);
            index(&self.subrs)
        };
        private.extend_from_slice(&self.private_padding);
        let name_index = index(&[self.font_name.as_bytes().to_vec()]);
        let strings = index(
            &custom
                .iter()
                .map(|s| s.as_bytes().to_vec())
                .collect::<Vec<_>>(),
        );
        let gsubrs = index(&self.gsubrs);
        let n_glyphs = (self.glyphs.len() + 1) as u16;
        let layout = |top_len: usize| {
            let mut off =
                4 + name_index.len() + index_len(&[top_len]) + strings.len() + gsubrs.len();
            let charset_off = off;
            off += charset.len();
            let encoding_off = off;
            off += encoding.as_ref().map_or(0, Vec::len);
            let charstrings_off = off;
            off += charstrings.len();
            let private_off = off;
            off += private.len() + subrs.len();
            let fdarray_off = off;
            off += index_len(&[font_dict(0, 0).len()]);
            (
                charset_off,
                encoding_off,
                charstrings_off,
                private_off,
                fdarray_off,
                off,
            )
        };
        let top = |l: (usize, usize, usize, usize, usize, usize)| {
            let (
                charset_off,
                encoding_off,
                charstrings_off,
                private_off,
                fdarray_off,
                fdselect_off,
            ) = l;
            let mut d = Vec::new();
            if self.cid_keyed {
                d.extend(int(ros_registry as i32));
                d.extend(int(ros_ordering as i32));
                d.extend(int(0));
                d.extend([12, 30]);
            }
            d.extend(int(charset_off as i32));
            d.push(15);
            if !self.cid_keyed {
                let enc = match (&self.encoding, &encoding) {
                    (CffEncodingSpec::Expert, _) => 1,
                    (_, Some(_)) => encoding_off as i32,
                    _ => 0,
                };
                d.extend(int(enc));
                d.push(16);
            }
            d.extend(int(charstrings_off as i32));
            d.push(17);
            if self.cid_keyed {
                d.extend(int(fdarray_off as i32));
                d.extend([12, 36]);
                d.extend(int(fdselect_off as i32));
                d.extend([12, 37]);
            } else {
                d.extend(int(private.len() as i32));
                d.extend(int(private_off as i32));
                d.push(18);
            }
            d.extend_from_slice(&self.top_padding);
            d
        };
        let top_len = top((0, 0, 0, 0, 0, 0)).len();
        let l = layout(top_len);
        let top_dict = top(l);
        assert_eq!(top_dict.len(), top_len);
        let mut out = vec![1, 0, 4, 4];
        out.extend(&name_index);
        out.extend(index(&[top_dict]));
        out.extend(&strings);
        out.extend(&gsubrs);
        assert_eq!(out.len(), l.0);
        out.extend(&charset);
        if let Some(e) = &encoding {
            out.extend(e);
        }
        out.extend(&charstrings);
        out.extend(&private);
        out.extend(&subrs);
        out.extend(index(&[font_dict(private.len(), l.3)]));
        if self.fdselect_format == 0 {
            // FDSelect format 0: FD 0 for every glyph
            out.push(0);
            out.extend(std::iter::repeat(0u8).take(usize::from(n_glyphs)));
        } else {
            // FDSelect format 3: one range, all glyphs in FD 0
            out.push(3);
            out.extend_from_slice(&1u16.to_be_bytes());
            out.extend_from_slice(&0u16.to_be_bytes());
            out.push(0);
            out.extend_from_slice(&n_glyphs.to_be_bytes());
        }
        out
    }
}

/// A Font DICT (FDArray entry) pointing at a Private DICT.
fn font_dict(private_len: usize, private_off: usize) -> Vec<u8> {
    let mut d = int(private_len as i32);
    d.extend(int(private_off as i32));
    d.push(18);
    d
}

/// A fixed-width (5-byte) DICT integer, so offsets can be patched without moving data.
fn int(v: i32) -> Vec<u8> {
    let mut b = vec![29];
    b.extend_from_slice(&v.to_be_bytes());
    b
}

/// A Type2 number.
pub fn t2(v: i32) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => {
            let w = v - 108;
            vec![(w / 256 + 247) as u8, (w % 256) as u8]
        }
        -1131..=-108 => {
            let w = -v - 108;
            vec![(w / 256 + 251) as u8, (w % 256) as u8]
        }
        _ => {
            let mut b = vec![28];
            b.extend_from_slice(&(v as i16).to_be_bytes());
            b
        }
    }
}

/// `100 0 rmoveto 400 0 0 500 -400 0 rlineto endchar`: a drawn box.
pub fn box_charstring() -> Vec<u8> {
    let mut c = Vec::new();
    for v in [100, 0] {
        c.extend(t2(v));
    }
    c.push(21);
    for v in [400, 0, 0, 500, -400, 0] {
        c.extend(t2(v));
    }
    c.push(5);
    c.push(14);
    c
}

/// A CFF INDEX with offSize 4.
pub fn index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    out.push(4);
    let mut off: u32 = 1;
    out.extend_from_slice(&off.to_be_bytes());
    for item in items {
        off += item.len() as u32;
        out.extend_from_slice(&off.to_be_bytes());
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

fn index_len(lens: &[usize]) -> usize {
    if lens.is_empty() {
        return 2;
    }
    3 + 4 * (lens.len() + 1) + lens.iter().sum::<usize>()
}
