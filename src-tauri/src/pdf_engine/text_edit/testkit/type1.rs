//! Type1 font program builder for tests (T2, SPEC §E.1): a PDF `FontFile` (clear text +
//! eexec-encrypted private part + 512-zero trailer, with `Length1/2/3`), binary or hex eexec,
//! `/lenIV` 4 or −1, `/Subrs` with the usual hint-replacement entries, and charstrings that draw,
//! draw nothing, use hints/subroutines or compose an accent with `seac`.

/// One charstring kind.
#[derive(Debug, Clone)]
pub enum T1Glyph {
    /// `hsbw`, a closed box, `endchar`.
    Box { width: i32 },
    /// The same box behind `hstem`/`vstem` hints and a hint-replacement subroutine call.
    HintedBox { width: i32 },
    /// `hsbw … endchar` with nothing drawn (whitespace).
    Blank { width: i32 },
    /// `seac` composite of two StandardEncoding codes.
    Seac { width: i32, base: u8, accent: u8 },
    /// Plain (unencrypted) charstring bytes, used as given.
    Raw(Vec<u8>),
}

#[derive(Debug, Clone)]
pub enum T1Encoding {
    Standard,
    Custom(Vec<(u8, String)>),
}

#[derive(Debug, Clone)]
pub struct Type1Builder {
    pub font_name: String,
    pub encoding: T1Encoding,
    pub glyphs: Vec<(String, T1Glyph)>,
    /// `/lenIV` (4 = default, −1 = charstrings not encrypted).
    pub len_iv: i64,
    pub hex: bool,
    /// Plain subroutines appended after the five standard ones (indices 5, 6, …).
    pub extra_subrs: Vec<Vec<u8>>,
}

/// A built `FontFile`: data and its `Length1`/`Length2`/`Length3`.
#[derive(Debug, Clone)]
pub struct Type1File {
    pub data: Vec<u8>,
    pub length1: usize,
    pub length2: usize,
    pub length3: usize,
}

impl Type1Builder {
    pub fn new(font_name: &str) -> Self {
        Type1Builder {
            font_name: font_name.to_string(),
            encoding: T1Encoding::Standard,
            glyphs: vec![(".notdef".to_string(), T1Glyph::Blank { width: 0 })],
            len_iv: 4,
            hex: false,
            extra_subrs: Vec::new(),
        }
    }

    pub fn glyph(mut self, name: &str, glyph: T1Glyph) -> Self {
        self.glyphs.push((name.to_string(), glyph));
        self
    }

    pub fn build(&self) -> Type1File {
        let mut clear = format!(
            "%!PS-AdobeFont-1.0: {name} 001.000\n%%Title: {name}\n11 dict begin\n\
             /FontInfo 2 dict dup begin\n/FullName ({name} \\(test\\)) readonly def\n\
             /ItalicAngle 0 def\nend readonly def\n/FontName /{name} def\n",
            name = self.font_name
        );
        match &self.encoding {
            T1Encoding::Standard => clear.push_str("/Encoding StandardEncoding def\n"),
            T1Encoding::Custom(codes) => {
                clear.push_str("/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n");
                for (code, name) in codes {
                    clear.push_str(&format!("dup {code} /{name} put\n"));
                }
                clear.push_str("readonly def\n");
            }
        }
        clear.push_str(
            "/PaintType 0 def\n/FontType 1 def\n/FontMatrix [0.001 0 0 0.001 0 0] readonly def\n\
             /FontBBox {0 -200 1000 800} readonly def\ncurrentdict end\ncurrentfile eexec\n",
        );
        let mut private = Vec::new();
        private.extend_from_slice(
            b"dup /Private 8 dict dup begin\n/RD {string currentfile exch readstring pop} executeonly def\n\
              /ND {noaccess def} executeonly def\n/NP {noaccess put} executeonly def\n",
        );
        private.extend_from_slice(format!("/lenIV {} def\n", self.len_iv).as_bytes());
        private.extend_from_slice(
            b"/BlueValues [-15 0 700 715] def\n/MinFeature {16 16} def\n/password 5839 def\n",
        );
        let mut subrs = standard_subrs();
        subrs.extend(self.extra_subrs.iter().cloned());
        private.extend_from_slice(format!("/Subrs {} array\n", subrs.len()).as_bytes());
        for (i, s) in subrs.iter().enumerate() {
            let enc = self.charstring(s);
            private.extend_from_slice(format!("dup {i} {} RD ", enc.len()).as_bytes());
            private.extend_from_slice(&enc);
            private.extend_from_slice(b" NP\n");
        }
        private.extend_from_slice(b"ND\n");
        private.extend_from_slice(
            format!(
                "2 index /CharStrings {} dict dup begin\n",
                self.glyphs.len()
            )
            .as_bytes(),
        );
        for (name, glyph) in &self.glyphs {
            let enc = self.charstring(&plain(glyph));
            private.extend_from_slice(format!("/{name} {} -| ", enc.len()).as_bytes());
            private.extend_from_slice(&enc);
            private.extend_from_slice(b" |-\n");
        }
        private.extend_from_slice(
            b"end\nend\nreadonly put\nnoaccess put\ndup /FontName get exch definefont pop\n\
              mark currentfile closefile\n",
        );
        let mut plaintext = vec![0u8; 4];
        plaintext.extend(private);
        let cipher = encrypt(&plaintext, 55665);
        let encrypted = if self.hex {
            let mut h = Vec::new();
            for (i, b) in cipher.iter().enumerate() {
                h.extend_from_slice(format!("{b:02x}").as_bytes());
                if i % 32 == 31 {
                    h.push(b'\n');
                }
            }
            h.push(b'\n');
            h
        } else {
            cipher
        };
        let mut trailer = Vec::new();
        for _ in 0..8 {
            trailer.extend_from_slice(&[b'0'; 64]);
            trailer.push(b'\n');
        }
        trailer.extend_from_slice(b"cleartomark\n");
        let mut data = clear.into_bytes();
        let length1 = data.len();
        data.extend(&encrypted);
        data.extend(&trailer);
        Type1File {
            data,
            length1,
            length2: encrypted.len(),
            length3: trailer.len(),
        }
    }

    /// Encrypts a plain charstring (r = 4330, `lenIV` zero bytes in front) unless lenIV is −1.
    fn charstring(&self, plain: &[u8]) -> Vec<u8> {
        if self.len_iv < 0 {
            return plain.to_vec();
        }
        let mut p = vec![0u8; self.len_iv as usize];
        p.extend_from_slice(plain);
        encrypt(&p, 4330)
    }
}

/// Type1 encryption (inverse of `fonts::type1::decrypt`).
pub fn encrypt(plain: &[u8], r: u16) -> Vec<u8> {
    let mut r = r;
    plain
        .iter()
        .map(|p| {
            let c = p ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r))
                .wrapping_mul(52845)
                .wrapping_add(22719);
            c
        })
        .collect()
}

/// A Type1 charstring number.
pub fn num(v: i32) -> Vec<u8> {
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
            let mut b = vec![255];
            b.extend_from_slice(&v.to_be_bytes());
            b
        }
    }
}

/// Numbers followed by an operator byte (or `12 x`).
pub fn op(args: &[i32], code: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = args.iter().flat_map(|a| num(*a)).collect();
    out.extend_from_slice(code);
    out
}

fn closed_box() -> Vec<u8> {
    [
        op(&[100, 0], &[21]),
        op(&[400, 0], &[5]),
        op(&[0, 500], &[5]),
        op(&[-400, 0], &[5]),
        vec![9],
    ]
    .concat()
}

/// The plain charstring of a glyph kind.
pub fn plain(glyph: &T1Glyph) -> Vec<u8> {
    match glyph {
        T1Glyph::Box { width } => [op(&[0, *width], &[13]), closed_box(), vec![14]].concat(),
        T1Glyph::HintedBox { width } => [
            op(&[0, *width], &[13]),
            op(&[0, 50], &[1]),        // hstem
            op(&[100, 50], &[3]),      // vstem
            op(&[4, 1, 3], &[12, 16]), // hint replacement: subr 4 through othersubr 3 …
            vec![12, 17],              // … pop
            vec![10],                  // … callsubr
            closed_box(),
            vec![14],
        ]
        .concat(),
        T1Glyph::Blank { width } => [op(&[0, *width], &[13]), vec![14]].concat(),
        T1Glyph::Seac {
            width,
            base,
            accent,
        } => [
            op(&[0, *width], &[13]),
            op(&[0, 0, 0, i32::from(*base), i32::from(*accent)], &[12, 6]),
        ]
        .concat(),
        T1Glyph::Raw(bytes) => bytes.clone(),
    }
}

/// Subrs 0–3 as Adobe's fonts carry them (flex/hint helpers) plus subr 4, a hint replacement.
fn standard_subrs() -> Vec<Vec<u8>> {
    vec![
        [
            op(&[3, 0], &[12, 16]),
            vec![12, 17, 12, 17],
            vec![12, 33],
            vec![11],
        ]
        .concat(),
        [op(&[0, 1], &[12, 16]), vec![11]].concat(),
        [op(&[0, 2], &[12, 16]), vec![11]].concat(),
        vec![11],
        [op(&[0, 60], &[1]), vec![11]].concat(),
    ]
}
