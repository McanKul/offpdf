//! TrueType / OpenType font program builder for tests (T2, SPEC §E.1): `head`, `hhea`, `maxp`,
//! `hmtx`, `cmap` ((3,1) and (3,0) format 4, (1,0) format 0), `post` (format 2 names or format 3),
//! `glyf` + `loca` (one triangle per drawn glyph, empty entries for blank ones — the shape of a
//! "zeroed" Word subset — or a raw glyph record such as a composite) or, for OpenType-CFF
//! (`OTTO`), a `CFF ` (or `CFF2`) table instead.

/// The 258 standard Macintosh glyph names of `post` format 2 (Apple TrueType Reference Manual), as
/// listed by ttf-parser 0.25.1 `src/tables/post.rs` (MIT OR Apache-2.0). A standard name must be
/// stored by its index: readers look standard names up by index only.
#[rustfmt::skip]
pub const MACINTOSH_NAMES: [&str; 258] = [
    ".notdef", ".null", "nonmarkingreturn", "space", "exclam", "quotedbl", "numbersign", "dollar",
    "percent", "ampersand", "quotesingle", "parenleft", "parenright", "asterisk", "plus", "comma",
    "hyphen", "period", "slash", "zero", "one", "two", "three", "four", "five", "six", "seven",
    "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question", "at", "A", "B",
    "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U",
    "V", "W", "X", "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum",
    "underscore", "grave", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n",
    "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "braceleft", "bar", "braceright",
    "asciitilde", "Adieresis", "Aring", "Ccedilla", "Eacute", "Ntilde", "Odieresis", "Udieresis",
    "aacute", "agrave", "acircumflex", "adieresis", "atilde", "aring", "ccedilla", "eacute",
    "egrave", "ecircumflex", "edieresis", "iacute", "igrave", "icircumflex", "idieresis", "ntilde",
    "oacute", "ograve", "ocircumflex", "odieresis", "otilde", "uacute", "ugrave", "ucircumflex",
    "udieresis", "dagger", "degree", "cent", "sterling", "section", "bullet", "paragraph",
    "germandbls", "registered", "copyright", "trademark", "acute", "dieresis", "notequal", "AE",
    "Oslash", "infinity", "plusminus", "lessequal", "greaterequal", "yen", "mu", "partialdiff",
    "summation", "product", "pi", "integral", "ordfeminine", "ordmasculine", "Omega", "ae",
    "oslash", "questiondown", "exclamdown", "logicalnot", "radical", "florin", "approxequal",
    "Delta", "guillemotleft", "guillemotright", "ellipsis", "nonbreakingspace", "Agrave", "Atilde",
    "Otilde", "OE", "oe", "endash", "emdash", "quotedblleft", "quotedblright", "quoteleft",
    "quoteright", "divide", "lozenge", "ydieresis", "Ydieresis", "fraction", "currency",
    "guilsinglleft", "guilsinglright", "fi", "fl", "daggerdbl", "periodcentered", "quotesinglbase",
    "quotedblbase", "perthousand", "Acircumflex", "Ecircumflex", "Aacute", "Edieresis", "Egrave",
    "Iacute", "Icircumflex", "Idieresis", "Igrave", "Oacute", "Ocircumflex", "apple", "Ograve",
    "Uacute", "Ucircumflex", "Ugrave", "dotlessi", "circumflex", "tilde", "macron", "breve",
    "dotaccent", "ring", "cedilla", "hungarumlaut", "ogonek", "caron", "Lslash", "lslash", "Scaron",
    "scaron", "Zcaron", "zcaron", "brokenbar", "Eth", "eth", "Yacute", "yacute", "Thorn", "thorn",
    "minus", "multiply", "onesuperior", "twosuperior", "threesuperior", "onehalf", "onequarter",
    "threequarters", "franc", "Gbreve", "gbreve", "Idotaccent", "Scedilla", "scedilla", "Cacute",
    "cacute", "Ccaron", "ccaron", "dcroat",
];

#[derive(Debug, Clone)]
pub struct TtfGlyph {
    pub name: String,
    pub outline: bool,
    pub advance: u16,
    /// A raw `glyf` record used instead of the triangle (e.g. a composite).
    pub data: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct TtfBuilder {
    /// GID 1, 2, … (GID 0 is a drawn `.notdef`).
    pub glyphs: Vec<TtfGlyph>,
    pub cmap31: Vec<(u32, u16)>,
    pub cmap30: Vec<(u32, u16)>,
    pub cmap10: Vec<(u8, u16)>,
    pub post_names: bool,
    pub units_per_em: u16,
    pub ascender: i16,
    pub descender: i16,
    /// OpenType-CFF: this `CFF ` table replaces `glyf`/`loca`.
    pub cff: Option<Vec<u8>>,
    /// OpenType-CFF2: this `CFF2` table replaces `glyf`/`loca`.
    pub cff2: Option<Vec<u8>>,
}

impl Default for TtfBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TtfBuilder {
    pub fn new() -> Self {
        TtfBuilder {
            glyphs: Vec::new(),
            cmap31: Vec::new(),
            cmap30: Vec::new(),
            cmap10: Vec::new(),
            post_names: true,
            units_per_em: 1000,
            ascender: 800,
            descender: -200,
            cff: None,
            cff2: None,
        }
    }

    /// Adds a glyph and returns its GID.
    pub fn glyph(&mut self, name: &str, outline: bool, advance: u16) -> u16 {
        self.glyphs.push(TtfGlyph {
            name: name.to_string(),
            outline,
            advance,
            data: None,
        });
        self.glyphs.len() as u16
    }

    /// Adds a glyph with a raw `glyf` record (see `composite`) and returns its GID.
    pub fn raw_glyph(&mut self, name: &str, data: Vec<u8>, advance: u16) -> u16 {
        self.glyphs.push(TtfGlyph {
            name: name.to_string(),
            outline: true,
            advance,
            data: Some(data),
        });
        self.glyphs.len() as u16
    }

    /// A glyph mapped from `ch` in the (3,1) cmap; returns its GID.
    pub fn unicode_glyph(&mut self, ch: char, name: &str, outline: bool) -> u16 {
        let gid = self.glyph(name, outline, 500);
        self.cmap31.push((u32::from(ch), gid));
        gid
    }

    pub fn number_of_glyphs(&self) -> u16 {
        self.glyphs.len() as u16 + 1
    }

    /// A non-conformant sfnt (review T4 r2 M-2) holding both outline formats: `glyf`/`loca` from
    /// the glyphs and the `CFF ` table `cff`, under the sfnt version `magic` (`OTTO` or 1.0).
    pub fn build_glyf_and_cff(&self, cff: Vec<u8>, magic: u32) -> Vec<u8> {
        let glyf_only = TtfBuilder {
            cff: None,
            cff2: None,
            ..self.clone()
        };
        let font = glyf_only.build();
        let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
        let mut tables: Vec<([u8; 4], Vec<u8>)> = (0..count)
            .map(|i| {
                let r = 12 + 16 * i;
                let field = |k: usize| {
                    u32::from_be_bytes([
                        font[r + k],
                        font[r + k + 1],
                        font[r + k + 2],
                        font[r + k + 3],
                    ]) as usize
                };
                let tag = [font[r], font[r + 1], font[r + 2], font[r + 3]];
                let (offset, len) = (field(8), field(12));
                (tag, font[offset..offset + len].to_vec())
            })
            .collect();
        tables.push((*b"CFF ", cff));
        tables.sort_by(|a, b| a.0.cmp(&b.0));
        sfnt(magic, &tables)
    }

    pub fn build(&self) -> Vec<u8> {
        let n = self.number_of_glyphs();
        let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
        tables.push((*b"head", self.head()));
        tables.push((*b"hhea", self.hhea(n)));
        tables.push((
            *b"maxp",
            [0x0000_5000u32.to_be_bytes().as_slice(), &n.to_be_bytes()].concat(),
        ));
        tables.push((*b"hmtx", self.hmtx()));
        tables.push((*b"cmap", self.cmap()));
        tables.push((*b"post", self.post()));
        match (&self.cff, &self.cff2) {
            (Some(cff), _) => tables.push((*b"CFF ", cff.clone())),
            (None, Some(cff2)) => tables.push((*b"CFF2", cff2.clone())),
            (None, None) => {
                let (glyf, loca) = self.glyf_loca();
                tables.push((*b"glyf", glyf));
                tables.push((*b"loca", loca));
            }
        }
        tables.sort_by(|a, b| a.0.cmp(&b.0));
        let magic: u32 = if self.cff.is_some() || self.cff2.is_some() {
            0x4F54_544F
        } else {
            0x0001_0000
        };
        sfnt(magic, &tables)
    }

    fn head(&self) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        h.extend_from_slice(&0u32.to_be_bytes());
        h.extend_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        h.extend_from_slice(&0u16.to_be_bytes());
        h.extend_from_slice(&self.units_per_em.to_be_bytes());
        h.extend_from_slice(&[0; 16]);
        for v in [0i16, self.descender, 1000, self.ascender] {
            h.extend_from_slice(&v.to_be_bytes());
        }
        h.extend_from_slice(&[0, 0, 0, 8, 0, 2]);
        h.extend_from_slice(&1u16.to_be_bytes()); // long loca
        h.extend_from_slice(&0u16.to_be_bytes());
        h
    }

    fn hhea(&self, n: u16) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        h.extend_from_slice(&self.ascender.to_be_bytes());
        h.extend_from_slice(&self.descender.to_be_bytes());
        h.extend_from_slice(&[0; 26]);
        h.extend_from_slice(&n.to_be_bytes());
        h
    }

    fn hmtx(&self) -> Vec<u8> {
        let mut h = Vec::new();
        h.extend_from_slice(&500u16.to_be_bytes());
        h.extend_from_slice(&0i16.to_be_bytes());
        for g in &self.glyphs {
            h.extend_from_slice(&g.advance.to_be_bytes());
            h.extend_from_slice(&0i16.to_be_bytes());
        }
        h
    }

    fn glyf_loca(&self) -> (Vec<u8>, Vec<u8>) {
        let mut glyf = Vec::new();
        let mut loca = vec![0u32];
        let records = std::iter::once((true, None))
            .chain(self.glyphs.iter().map(|g| (g.outline, g.data.as_ref())));
        for (outline, data) in records {
            match data {
                Some(data) => {
                    glyf.extend(data);
                    while glyf.len() % 4 != 0 {
                        glyf.push(0);
                    }
                }
                None if outline => glyf.extend(triangle()),
                None => {}
            }
            loca.push(glyf.len() as u32);
        }
        (glyf, loca.iter().flat_map(|o| o.to_be_bytes()).collect())
    }

    fn cmap(&self) -> Vec<u8> {
        let mut subtables: Vec<(u16, u16, Vec<u8>)> = Vec::new();
        if !self.cmap10.is_empty() {
            let mut ids = [0u8; 256];
            for (code, gid) in &self.cmap10 {
                ids[usize::from(*code)] = *gid as u8;
            }
            let mut t = Vec::new();
            t.extend_from_slice(&0u16.to_be_bytes());
            t.extend_from_slice(&262u16.to_be_bytes());
            t.extend_from_slice(&0u16.to_be_bytes());
            t.extend_from_slice(&ids);
            subtables.push((1, 0, t));
        }
        if !self.cmap30.is_empty() {
            subtables.push((3, 0, format4(&self.cmap30)));
        }
        if !self.cmap31.is_empty() {
            subtables.push((3, 1, format4(&self.cmap31)));
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_be_bytes());
        out.extend_from_slice(&(subtables.len() as u16).to_be_bytes());
        let mut offset = 4 + 8 * subtables.len();
        for (platform, encoding, data) in &subtables {
            out.extend_from_slice(&platform.to_be_bytes());
            out.extend_from_slice(&encoding.to_be_bytes());
            out.extend_from_slice(&(offset as u32).to_be_bytes());
            offset += data.len();
        }
        for (_, _, data) in &subtables {
            out.extend_from_slice(data);
        }
        out
    }

    fn post(&self) -> Vec<u8> {
        let mut p = Vec::new();
        let version: u32 = if self.post_names {
            0x0002_0000
        } else {
            0x0003_0000
        };
        p.extend_from_slice(&version.to_be_bytes());
        p.extend_from_slice(&[0; 28]);
        if !self.post_names {
            return p;
        }
        p.extend_from_slice(&self.number_of_glyphs().to_be_bytes());
        p.extend_from_slice(&0u16.to_be_bytes()); // .notdef = standard Mac name 0
        let mut custom: Vec<&str> = Vec::new();
        for g in &self.glyphs {
            let index = match MACINTOSH_NAMES.iter().position(|n| *n == g.name) {
                Some(i) => i as u16,
                None => {
                    custom.push(&g.name);
                    257 + custom.len() as u16
                }
            };
            p.extend_from_slice(&index.to_be_bytes());
        }
        for name in custom {
            p.push(name.len() as u8);
            p.extend_from_slice(name.as_bytes());
        }
        p
    }
}

/// A one-contour triangle (3 on-curve points).
fn triangle() -> Vec<u8> {
    let mut g = Vec::new();
    for v in [1i16, 50, 0, 450, 600] {
        g.extend_from_slice(&v.to_be_bytes()); // numberOfContours, xMin, yMin, xMax, yMax
    }
    g.extend_from_slice(&2u16.to_be_bytes()); // endPtsOfContours
    g.extend_from_slice(&0u16.to_be_bytes()); // instructionLength
    g.extend_from_slice(&[1, 1, 1]); // flags: on-curve, 2-byte deltas
    for dx in [50i16, 400, -200] {
        g.extend_from_slice(&dx.to_be_bytes());
    }
    for dy in [0i16, 0, 600] {
        g.extend_from_slice(&dy.to_be_bytes());
    }
    while g.len() % 4 != 0 {
        g.push(0);
    }
    g
}

/// A composite `glyf` record placing each of `components` (GIDs) with byte offsets.
pub fn composite(components: &[u16]) -> Vec<u8> {
    let mut g = Vec::new();
    for v in [-1i16, 0, 0, 1000, 1000] {
        g.extend_from_slice(&v.to_be_bytes()); // numberOfContours −1, bbox
    }
    for (i, gid) in components.iter().enumerate() {
        let more = if i + 1 < components.len() { 0x0020 } else { 0 };
        let flags: u16 = 0x0002 | more; // ARGS_ARE_XY_VALUES, byte offsets
        g.extend_from_slice(&flags.to_be_bytes());
        g.extend_from_slice(&gid.to_be_bytes());
        g.extend_from_slice(&[i as u8, 0]);
    }
    g
}

/// cmap format 4 with one segment per mapping (+ the final 0xFFFF segment).
fn format4(map: &[(u32, u16)]) -> Vec<u8> {
    let mut pairs: Vec<(u16, u16)> = map.iter().map(|(c, g)| (*c as u16, *g)).collect();
    pairs.sort();
    pairs.dedup_by_key(|p| p.0);
    pairs.push((0xFFFF, 0));
    let seg = pairs.len() as u16;
    let mut t = Vec::new();
    t.extend_from_slice(&4u16.to_be_bytes());
    t.extend_from_slice(&(16 + 8 * seg).to_be_bytes());
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&(seg * 2).to_be_bytes());
    t.extend_from_slice(&[0; 6]); // searchRange, entrySelector, rangeShift (unused by readers here)
    for (c, _) in &pairs {
        t.extend_from_slice(&c.to_be_bytes());
    }
    t.extend_from_slice(&0u16.to_be_bytes());
    for (c, _) in &pairs {
        t.extend_from_slice(&c.to_be_bytes());
    }
    for (c, g) in &pairs {
        let delta = if *c == 0xFFFF {
            1u16
        } else {
            g.wrapping_sub(*c)
        };
        t.extend_from_slice(&delta.to_be_bytes());
    }
    for _ in &pairs {
        t.extend_from_slice(&0u16.to_be_bytes());
    }
    t
}

/// An sfnt wrapper: table directory (sorted by tag) and 4-byte aligned tables.
pub fn sfnt(magic: u32, tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&magic.to_be_bytes());
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    let mut offset = 12 + 16 * tables.len();
    let mut body = Vec::new();
    for (tag, data) in tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut padded = data.clone();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        offset += padded.len();
        body.extend(padded);
    }
    out.extend(body);
    out
}
