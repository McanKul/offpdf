//! The layout of a CFF (version 1) program exactly as ttf-parser 0.25.1 reads it
//! (`tables/cff/{cff1,index,dict,charset}.rs`, MIT OR Apache-2.0): INDEX and DICT structures, the
//! Top DICT offsets, the global and local subroutines (FDSelect → Font DICT → Private DICT for
//! CID-keyed fonts) and the charset. ttf-parser keeps all of them crate-private, but
//! - the bounded charstring pre-check (`glyph_budget.rs`) must walk exactly the bytes
//!   `cff::Table::outline` would run, so it follows every rule here that decides which bytes those
//!   are (offsets taken as ttf-parser takes them, later Top DICT entries winning, the first
//!   Font DICT `Private` entry, FDSelect formats 0 and 3, the seac charset lookup);
//! - ttf-parser walks a format 1/2 charset once per glyph (`glyph_name`, `glyph_cid`), which is
//!   quadratic over a whole font; `gid_to_sid` inverts it once.
//!
//! A program this reader cannot follow (a real-number offset, or a structure it rejects that
//! ttf-parser accepted) is `None`; callers then fail closed. Checked slicing only.
//!
//! DICTs are walked lazily (no entry list is built) and only up to `DICT_LEN_MAX` bytes: a Top
//! or name-keyed Private DICT past it makes the program unreadable, a CID-keyed font's Font or
//! Private DICT past it gives that Font DICT no local subroutines (its glyphs that call one do not
//! draw). ttf-parser walks a CID-keyed font's Font and Private DICT again for every glyph it
//! outlines that calls a local subroutine, so `FdSubrs::dict_bytes` reports their size for the
//! pre-check to charge per glyph.

use std::collections::HashMap;
use std::ops::Range;

/// The 391 CFF standard strings (Adobe TN #5176 Appendix A), as ttf-parser 0.25.1
/// `src/tables/cff/std_names.rs` lists them (MIT OR Apache-2.0). SID n < 391 names
/// `STANDARD_STRINGS[n]`; higher SIDs index the String INDEX.
#[rustfmt::skip]
pub(crate) const STANDARD_STRINGS: [&str; 391] = [
    ".notdef", "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand",
    "quoteright", "parenleft", "parenright", "asterisk", "plus", "comma", "hyphen", "period",
    "slash", "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    "colon", "semicolon", "less", "equal", "greater", "question", "at", "A", "B", "C", "D", "E",
    "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X",
    "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "quoteleft",
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z", "braceleft", "bar", "braceright", "asciitilde", "exclamdown",
    "cent", "sterling", "fraction", "yen", "florin", "section", "currency", "quotesingle",
    "quotedblleft", "guillemotleft", "guilsinglleft", "guilsinglright", "fi", "fl", "endash",
    "dagger", "daggerdbl", "periodcentered", "paragraph", "bullet", "quotesinglbase",
    "quotedblbase", "quotedblright", "guillemotright", "ellipsis", "perthousand", "questiondown",
    "grave", "acute", "circumflex", "tilde", "macron", "breve", "dotaccent", "dieresis", "ring",
    "cedilla", "hungarumlaut", "ogonek", "caron", "emdash", "AE", "ordfeminine", "Lslash", "Oslash",
    "OE", "ordmasculine", "ae", "dotlessi", "lslash", "oslash", "oe", "germandbls", "onesuperior",
    "logicalnot", "mu", "trademark", "Eth", "onehalf", "plusminus", "Thorn", "onequarter", "divide",
    "brokenbar", "degree", "thorn", "threequarters", "twosuperior", "registered", "minus", "eth",
    "multiply", "threesuperior", "copyright", "Aacute", "Acircumflex", "Adieresis", "Agrave",
    "Aring", "Atilde", "Ccedilla", "Eacute", "Ecircumflex", "Edieresis", "Egrave", "Iacute",
    "Icircumflex", "Idieresis", "Igrave", "Ntilde", "Oacute", "Ocircumflex", "Odieresis", "Ograve",
    "Otilde", "Scaron", "Uacute", "Ucircumflex", "Udieresis", "Ugrave", "Yacute", "Ydieresis",
    "Zcaron", "aacute", "acircumflex", "adieresis", "agrave", "aring", "atilde", "ccedilla",
    "eacute", "ecircumflex", "edieresis", "egrave", "iacute", "icircumflex", "idieresis", "igrave",
    "ntilde", "oacute", "ocircumflex", "odieresis", "ograve", "otilde", "scaron", "uacute",
    "ucircumflex", "udieresis", "ugrave", "yacute", "ydieresis", "zcaron", "exclamsmall",
    "Hungarumlautsmall", "dollaroldstyle", "dollarsuperior", "ampersandsmall", "Acutesmall",
    "parenleftsuperior", "parenrightsuperior", "twodotenleader", "onedotenleader", "zerooldstyle",
    "oneoldstyle", "twooldstyle", "threeoldstyle", "fouroldstyle", "fiveoldstyle", "sixoldstyle",
    "sevenoldstyle", "eightoldstyle", "nineoldstyle", "commasuperior", "threequartersemdash",
    "periodsuperior", "questionsmall", "asuperior", "bsuperior", "centsuperior", "dsuperior",
    "esuperior", "isuperior", "lsuperior", "msuperior", "nsuperior", "osuperior", "rsuperior",
    "ssuperior", "tsuperior", "ff", "ffi", "ffl", "parenleftinferior", "parenrightinferior",
    "Circumflexsmall", "hyphensuperior", "Gravesmall", "Asmall", "Bsmall", "Csmall", "Dsmall",
    "Esmall", "Fsmall", "Gsmall", "Hsmall", "Ismall", "Jsmall", "Ksmall", "Lsmall", "Msmall",
    "Nsmall", "Osmall", "Psmall", "Qsmall", "Rsmall", "Ssmall", "Tsmall", "Usmall", "Vsmall",
    "Wsmall", "Xsmall", "Ysmall", "Zsmall", "colonmonetary", "onefitted", "rupiah", "Tildesmall",
    "exclamdownsmall", "centoldstyle", "Lslashsmall", "Scaronsmall", "Zcaronsmall", "Dieresissmall",
    "Brevesmall", "Caronsmall", "Dotaccentsmall", "Macronsmall", "figuredash", "hypheninferior",
    "Ogoneksmall", "Ringsmall", "Cedillasmall", "questiondownsmall", "oneeighth", "threeeighths",
    "fiveeighths", "seveneighths", "onethird", "twothirds", "zerosuperior", "foursuperior",
    "fivesuperior", "sixsuperior", "sevensuperior", "eightsuperior", "ninesuperior", "zeroinferior",
    "oneinferior", "twoinferior", "threeinferior", "fourinferior", "fiveinferior", "sixinferior",
    "seveninferior", "eightinferior", "nineinferior", "centinferior", "dollarinferior",
    "periodinferior", "commainferior", "Agravesmall", "Aacutesmall", "Acircumflexsmall",
    "Atildesmall", "Adieresissmall", "Aringsmall", "AEsmall", "Ccedillasmall", "Egravesmall",
    "Eacutesmall", "Ecircumflexsmall", "Edieresissmall", "Igravesmall", "Iacutesmall",
    "Icircumflexsmall", "Idieresissmall", "Ethsmall", "Ntildesmall", "Ogravesmall", "Oacutesmall",
    "Ocircumflexsmall", "Otildesmall", "Odieresissmall", "OEsmall", "Oslashsmall", "Ugravesmall",
    "Uacutesmall", "Ucircumflexsmall", "Udieresissmall", "Yacutesmall", "Thornsmall",
    "Ydieresissmall", "001.000", "001.001", "001.002", "001.003", "Black", "Bold", "Book", "Light",
    "Medium", "Regular", "Roman", "Semibold",
];

/// The CFF Standard Encoding, code → SID (Adobe TN #5176 Appendix B; ttf-parser 0.25.1
/// `encoding.rs` `STANDARD_ENCODING`), used by `seac`.
#[rustfmt::skip]
const STANDARD_ENCODING: [u8; 256] = [
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      1,   2,   3,   4,   5,   6,   7,   8,   9,  10,  11,  12,  13,  14,  15,  16,
     17,  18,  19,  20,  21,  22,  23,  24,  25,  26,  27,  28,  29,  30,  31,  32,
     33,  34,  35,  36,  37,  38,  39,  40,  41,  42,  43,  44,  45,  46,  47,  48,
     49,  50,  51,  52,  53,  54,  55,  56,  57,  58,  59,  60,  61,  62,  63,  64,
     65,  66,  67,  68,  69,  70,  71,  72,  73,  74,  75,  76,  77,  78,  79,  80,
     81,  82,  83,  84,  85,  86,  87,  88,  89,  90,  91,  92,  93,  94,  95,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0,  96,  97,  98,  99, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110,
      0, 111, 112, 113, 114,   0, 115, 116, 117, 118, 119, 120, 121, 122,   0, 123,
      0, 124, 125, 126, 127, 128, 129, 130, 131,   0, 132, 133,   0, 134, 135, 136,
    137,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,   0,
      0, 138,   0, 139,   0,   0,   0,   0, 140, 141, 142, 143,   0,   0,   0,   0,
      0, 144,   0,   0,   0, 145,   0,   0, 146, 147, 148, 149,   0,   0,   0,   0,
];

/// Operands ttf-parser keeps per DICT operator.
const DICT_OPERANDS_MAX: usize = 48;
/// Longest Top, Font or Private DICT followed (real ones are well under 1 KiB).
pub(crate) const DICT_LEN_MAX: usize = 16 << 10;
const OP_CHARSET: u16 = 15;
const OP_CHAR_STRINGS: u16 = 17;
const OP_PRIVATE: u16 = 18;
const OP_LOCAL_SUBRS: u16 = 19;
const OP_ROS: u16 = 1230;
const OP_FD_ARRAY: u16 = 1236;
const OP_FD_SELECT: u16 = 1237;

fn u16_at(data: &[u8], at: usize) -> Option<u16> {
    let bytes = data.get(at..at.checked_add(2)?)?;
    Some(u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]))
}

/// A CFF INDEX as ttf-parser's `parse_index::<u16>` builds it: offsets are 1-based and the data
/// runs to the last offset; a last offset of 0 makes an empty INDEX.
#[derive(Clone, Copy, Default)]
pub(crate) struct Index<'a> {
    data: &'a [u8],
    offsets: &'a [u8],
    off_size: usize,
}

impl<'a> Index<'a> {
    /// The INDEX at `at` and the offset right after it.
    fn parse(cff: &'a [u8], at: usize) -> Option<(Index<'a>, usize)> {
        let (offsets, off_size, pos) = match Self::header(cff, at)? {
            Header::Empty(pos) => return Some((Index::default(), pos)),
            Header::Offsets(offsets, off_size, pos) => (offsets, off_size, pos),
        };
        let shell = Index {
            data: &[],
            offsets,
            off_size,
        };
        match shell.offset(shell.offset_count().checked_sub(1)?) {
            Some(last) => {
                let end = pos.checked_add(last)?;
                let data = cff.get(pos..end)?;
                Some((Index { data, ..shell }, end))
            }
            None => Some((Index::default(), pos)),
        }
    }

    /// The offset after the INDEX at `at` (`skip_index`: the data is not bounds-checked).
    fn skip(cff: &'a [u8], at: usize) -> Option<usize> {
        match Self::header(cff, at)? {
            Header::Empty(pos) => Some(pos),
            Header::Offsets(offsets, off_size, pos) => {
                let shell = Index {
                    data: &[],
                    offsets,
                    off_size,
                };
                match shell.offset(shell.offset_count().checked_sub(1)?) {
                    Some(last) => pos.checked_add(last),
                    None => Some(pos),
                }
            }
        }
    }

    fn header(cff: &'a [u8], at: usize) -> Option<Header<'a>> {
        let count = u16_at(cff, at)?;
        let pos = at.checked_add(2)?;
        if count == 0 {
            return Some(Header::Empty(pos));
        }
        let off_size = usize::from(*cff.get(pos)?);
        if !(1..=4).contains(&off_size) {
            return None;
        }
        let pos = pos.checked_add(1)?;
        let len = (usize::from(count) + 1).checked_mul(off_size)?;
        let end = pos.checked_add(len)?;
        Some(Header::Offsets(cff.get(pos..end)?, off_size, end))
    }

    fn offset_count(&self) -> usize {
        self.offsets.len().checked_div(self.off_size).unwrap_or(0)
    }

    /// Offset `i` minus one (`None` past the end or for a raw 0).
    fn offset(&self, i: usize) -> Option<usize> {
        if i >= self.offset_count() {
            return None;
        }
        let at = i.checked_mul(self.off_size)?;
        let raw = self
            .offsets
            .get(at..at.checked_add(self.off_size)?)?
            .iter()
            .fold(0usize, |acc, b| (acc << 8) | usize::from(*b));
        raw.checked_sub(1)
    }

    /// Number of entries.
    pub fn len(&self) -> u32 {
        u32::try_from(self.offset_count().saturating_sub(1)).unwrap_or(u32::MAX)
    }

    pub fn get(&self, i: u32) -> Option<&'a [u8]> {
        let i = usize::try_from(i).ok()?;
        let start = self.offset(i)?;
        let end = self.offset(i.checked_add(1)?)?;
        self.data.get(start..end)
    }
}

enum Header<'a> {
    Empty(usize),
    Offsets(&'a [u8], usize, usize),
}

/// One DICT entry: the operator (two-byte operators as `1200 + b1`) and its operand bytes.
struct DictEntry<'a> {
    op: u16,
    operands: &'a [u8],
}

/// DICT entries in order, as `DictionaryParser::parse_next` finds them (operators 0–27, 31, 255;
/// the walk stops at the first number it cannot skip). Lazy: nothing is collected.
struct DictEntries<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> DictEntries<'a> {
    fn new(data: &'a [u8]) -> DictEntries<'a> {
        DictEntries { data, pos: 0 }
    }
}

impl<'a> Iterator for DictEntries<'a> {
    type Item = DictEntry<'a>;

    fn next(&mut self) -> Option<DictEntry<'a>> {
        let start = self.pos;
        while let Some(&b) = self.data.get(self.pos) {
            self.pos += 1;
            if matches!(b, 0..=27 | 31 | 255) {
                let op = if b == 12 {
                    let Some(&b1) = self.data.get(self.pos) else {
                        self.pos = self.data.len();
                        return None;
                    };
                    self.pos += 1;
                    1200 + u16::from(b1)
                } else {
                    u16::from(b)
                };
                let operands = self.data.get(start..self.pos).unwrap_or_default();
                return Some(DictEntry { op, operands });
            }
            match skip_number(b, self.data, self.pos) {
                Some(next) => self.pos = next,
                None => {
                    self.pos = self.data.len();
                    return None;
                }
            }
        }
        None
    }
}

/// `skip_number`: the offset after the operand starting with `b0` (unchecked advances, as there).
fn skip_number(b0: u8, data: &[u8], pos: usize) -> Option<usize> {
    match b0 {
        28 => pos.checked_add(2),
        29 => pos.checked_add(4),
        30 => {
            let mut pos = pos;
            while let Some(&b) = data.get(pos) {
                pos += 1;
                if b >> 4 == 0xF || b & 0xF == 0xF {
                    break;
                }
            }
            Some(pos)
        }
        32..=246 => Some(pos),
        247..=254 => pos.checked_add(1),
        _ => None,
    }
}

/// The integer operands of an entry (`parse_operands`, at most 48). `Err` for a real operand,
/// which this reader does not evaluate; `Ok(None)` where ttf-parser's parse fails.
fn int_operands(entry: &DictEntry<'_>) -> Result<Option<Vec<i64>>, ()> {
    let data = entry.operands;
    let mut out = Vec::new();
    let mut pos = 0usize;
    while let Some(&b0) = data.get(pos) {
        pos += 1;
        if matches!(b0, 0..=27 | 31 | 255) {
            break;
        }
        let at = pos;
        let value = match b0 {
            28 => {
                pos += 2;
                u16_at(data, at).map(|v| i64::from(v as i16))
            }
            29 => {
                pos += 4;
                u16_at(data, at)
                    .zip(u16_at(data, at + 2))
                    .map(|(hi, lo)| i64::from(((u32::from(hi) << 16) | u32::from(lo)) as i32))
            }
            30 => return Err(()),
            32..=246 => Some(i64::from(b0) - 139),
            247..=250 => {
                pos += 1;
                data.get(at)
                    .map(|b1| (i64::from(b0) - 247) * 256 + i64::from(*b1) + 108)
            }
            251..=254 => {
                pos += 1;
                data.get(at)
                    .map(|b1| -(i64::from(b0) - 251) * 256 - i64::from(*b1) - 108)
            }
            _ => None,
        };
        let Some(value) = value else {
            return Ok(None);
        };
        out.push(value);
        if out.len() >= DICT_OPERANDS_MAX {
            break;
        }
    }
    Ok(Some(out))
}

/// `parse_offset`: exactly one operand, as a non-negative `i32`.
fn dict_offset(entry: &DictEntry<'_>) -> Result<Option<usize>, ()> {
    Ok(int_operands(entry)?.and_then(|ops| match ops.as_slice() {
        [v] => i32::try_from(*v).ok().and_then(|v| usize::try_from(v).ok()),
        _ => None,
    }))
}

/// `parse_range`: `size offset` → `offset..offset + size`.
fn dict_range(entry: &DictEntry<'_>) -> Result<Option<Range<usize>>, ()> {
    Ok(int_operands(entry)?.and_then(|ops| match ops.as_slice() {
        [len, start] => {
            let len = usize::try_from(i32::try_from(*len).ok()?).ok()?;
            let start = usize::try_from(i32::try_from(*start).ok()?).ok()?;
            Some(start..start.checked_add(len)?)
        }
        _ => None,
    }))
}

/// The charset as ttf-parser parses it.
#[derive(Clone, Copy)]
pub(crate) enum Charset<'a> {
    IsoAdobe,
    /// Expert or Expert Subset (predefined; no `seac` lookup, names through ttf-parser).
    Expert,
    Format0(&'a [u8]),
    /// Ranges of `(first SID, nLeft)`; `wide` = format 2 (u16 nLeft).
    Ranges {
        data: &'a [u8],
        wide: bool,
    },
}

/// CID-keyed fonts: the FDArray and FDSelect.
#[derive(Clone, Copy)]
struct CidParts<'a> {
    fd_array: Index<'a>,
    fd_select: FdSelect<'a>,
}

#[derive(Clone, Copy)]
enum FdSelect<'a> {
    Format0(&'a [u8]),
    Format3(&'a [u8]),
}

/// Where a glyph's local subroutines come from.
pub(crate) enum LocalSubrs<'a> {
    /// Name-keyed fonts: the Private DICT's (possibly empty), laid out once.
    Font(Index<'a>),
    /// CID-keyed fonts: the Font DICT FDSelect selects (`None`: none), and the FDSelect ranges
    /// walked to find it.
    FontDict(Option<u8>, u64),
}

/// A CID-keyed font's Font DICT: its local subroutines and the DICT bytes walked to find them
/// (Font DICT + Private DICT), which ttf-parser walks again for every glyph it outlines that
/// calls a local subroutine.
#[derive(Clone, Copy)]
pub(crate) struct FdSubrs<'a> {
    pub subrs: Option<Index<'a>>,
    pub dict_bytes: u64,
}

/// What a CFF program's glyphs run: charstrings, global and local subroutines, and the charset.
pub(crate) struct CffLayout<'a> {
    data: &'a [u8],
    pub char_strings: Index<'a>,
    pub global_subrs: Index<'a>,
    strings: Index<'a>,
    pub charset: Charset<'a>,
    /// Name-keyed fonts: the Private DICT's local subroutines (possibly empty).
    sid_local_subrs: Index<'a>,
    cid: Option<CidParts<'a>>,
    pub number_of_glyphs: u16,
}

impl<'a> CffLayout<'a> {
    /// Mirrors `cff::Table::parse`; `None` where it fails or where this reader cannot follow.
    pub fn parse(data: &'a [u8]) -> Option<CffLayout<'a>> {
        if data.first() != Some(&1) {
            return None;
        }
        let header_size = usize::from(*data.get(2)?);
        let mut pos = 4usize;
        if header_size > 4 {
            pos = pos.checked_add(header_size - 4)?;
        }
        pos = Index::skip(data, pos)?; // Name INDEX
        let (top_index, after_top) = Index::parse(data, pos)?;
        let top = TopDict::parse(top_index.get(0)?)?;
        if top.char_strings == 0 {
            return None;
        }
        let (strings, after_strings) = Index::parse(data, after_top)?;
        let (global_subrs, _) = Index::parse(data, after_strings)?;
        if top.char_strings > data.len() {
            return None;
        }
        let (char_strings, _) = Index::parse(data, top.char_strings)?;
        let number_of_glyphs = u16::try_from(char_strings.len()).ok().filter(|n| *n > 0)?;
        let charset = match top.charset {
            Some(0) | None => Charset::IsoAdobe,
            Some(1 | 2) => Charset::Expert,
            Some(at) => parse_charset(data, at, number_of_glyphs)?,
        };
        let mut layout = CffLayout {
            data,
            char_strings,
            global_subrs,
            strings,
            charset,
            sid_local_subrs: Index::default(),
            cid: None,
            number_of_glyphs,
        };
        if top.ros {
            let (Some(charset_at), Some(fd_array_at), Some(fd_select_at)) =
                (top.charset, top.fd_array, top.fd_select)
            else {
                return None;
            };
            if charset_at <= 2 || fd_array_at > data.len() || fd_select_at > data.len() {
                return None;
            }
            let (fd_array, _) = Index::parse(data, fd_array_at)?;
            let fd_select = match *data.get(fd_select_at)? {
                0 => {
                    let start = fd_select_at.checked_add(1)?;
                    let end = start.checked_add(usize::from(number_of_glyphs))?;
                    FdSelect::Format0(data.get(start..end)?)
                }
                3 => FdSelect::Format3(data.get(fd_select_at.checked_add(1)?..)?),
                _ => return None,
            };
            layout.cid = Some(CidParts {
                fd_array,
                fd_select,
            });
        } else if let Some(range) = top.private {
            let private = PrivateDict::parse(data.get(range.clone())?)?;
            if let Some(start) = private
                .local_subrs
                .and_then(|off| range.start.checked_add(off))
            {
                layout.sid_local_subrs = Index::parse(data.get(start..)?, 0)?.0;
            }
        }
        Some(layout)
    }

    pub fn is_cid(&self) -> bool {
        self.cid.is_some()
    }

    /// Where the local subroutines `glyph` runs come from.
    pub fn local_subrs(&self, glyph: u16) -> LocalSubrs<'a> {
        match self.cid {
            None => LocalSubrs::Font(self.sid_local_subrs),
            Some(cid) => {
                let (fd, walked) = cid.fd_select.font_dict_index(glyph);
                LocalSubrs::FontDict(fd, walked)
            }
        }
    }

    /// The local subroutines of Font DICT `fd` as `parse_cid_local_subrs` finds them (Font DICT
    /// → its first `Private` entry → the Private DICT's `Subrs`), and the DICT bytes walked.
    pub fn fd_local_subrs(&self, fd: u8) -> FdSubrs<'a> {
        let mut out = FdSubrs {
            subrs: None,
            dict_bytes: 0,
        };
        let Some(font_dict) = self
            .cid
            .and_then(|cid| cid.fd_array.get(u32::from(fd)))
            .filter(|d| d.len() <= DICT_LEN_MAX)
        else {
            return out;
        };
        out.dict_bytes = font_dict.len() as u64;
        let Some(range) = DictEntries::new(font_dict)
            .find(|e| e.op == OP_PRIVATE)
            .and_then(|e| dict_range(&e).ok().flatten())
        else {
            return out;
        };
        let Some(private_data) = self.data.get(range.clone()) else {
            return out;
        };
        let Some(private) = PrivateDict::parse(private_data) else {
            return out; // past DICT_LEN_MAX (not walked), or an offset this reader cannot follow
        };
        out.dict_bytes = out.dict_bytes.saturating_add(private_data.len() as u64);
        out.subrs = private
            .local_subrs
            .and_then(|off| range.start.checked_add(off))
            .and_then(|start| Index::parse(self.data.get(start..)?, 0))
            .map(|(index, _)| index);
        out
    }

    /// GID → SID for every glyph, in one walk of the charset (`None` for the predefined Expert
    /// charsets, whose lookups ttf-parser answers in O(1) anyway).
    pub fn gid_to_sid(&self) -> Option<Vec<Option<u16>>> {
        let n = usize::from(self.number_of_glyphs);
        let mut out: Vec<Option<u16>> = Vec::with_capacity(n);
        out.push(Some(0));
        match self.charset {
            Charset::IsoAdobe => {
                out.extend((1..n).map(|gid| u16::try_from(gid).ok().filter(|g| *g <= 228)));
            }
            Charset::Expert => return None,
            Charset::Format0(sids) => {
                out.extend(sids.chunks_exact(2).map(|p| {
                    p.first()
                        .zip(p.get(1))
                        .map(|(h, l)| u16::from_be_bytes([*h, *l]))
                }));
            }
            Charset::Ranges { data, wide } => {
                let step = if wide { 4 } else { 3 };
                for range in data.chunks_exact(step) {
                    let first = u16_at(range, 0)?;
                    let left = if wide {
                        u16_at(range, 2)?
                    } else {
                        u16::from(*range.get(2)?)
                    };
                    for k in 0..=left {
                        if out.len() >= n {
                            break;
                        }
                        out.push(first.checked_add(k));
                    }
                }
            }
        }
        out.resize(n, None);
        Some(out)
    }

    /// The units ttf-parser's `sid_to_gid` walk costs once (format 0 scans every SID, formats
    /// 1/2 every range).
    pub fn charset_walk(&self) -> u64 {
        match self.charset {
            Charset::IsoAdobe | Charset::Expert => 1,
            Charset::Format0(sids) => (sids.len() / 2) as u64 + 1,
            Charset::Ranges { data, wide } => (data.len() / if wide { 4 } else { 3 }) as u64 + 1,
        }
    }

    /// The glyph name of SID `sid` (standard strings, then the String INDEX).
    pub fn sid_name(&self, sid: u16) -> Option<&'a str> {
        match STANDARD_STRINGS.get(usize::from(sid)) {
            Some(name) => Some(name),
            None => {
                let index = u32::from(sid).checked_sub(STANDARD_STRINGS.len() as u32)?;
                std::str::from_utf8(self.strings.get(index)?).ok()
            }
        }
    }
}

/// `seac`'s code → GID (`seac_code_to_glyph_id`): the Standard Encoding SID, then the charset.
pub(crate) fn seac_glyph(
    charset: Charset<'_>,
    sid_to_gid: &HashMap<u16, u16>,
    code: u8,
) -> Option<u16> {
    let sid = u16::from(*STANDARD_ENCODING.get(usize::from(code))?);
    match charset {
        Charset::IsoAdobe => (code <= 228).then_some(sid),
        Charset::Expert => None,
        Charset::Format0(_) | Charset::Ranges { .. } if sid == 0 => Some(0),
        Charset::Format0(_) | Charset::Ranges { .. } => sid_to_gid.get(&sid).copied(),
    }
}

/// The Top DICT entries ttf-parser reads (later entries win).
struct TopDict {
    charset: Option<usize>,
    char_strings: usize,
    private: Option<Range<usize>>,
    ros: bool,
    fd_array: Option<usize>,
    fd_select: Option<usize>,
}

impl TopDict {
    /// `None` past `DICT_LEN_MAX` or where an offset cannot be followed.
    fn parse(data: &[u8]) -> Option<TopDict> {
        if data.len() > DICT_LEN_MAX {
            return None;
        }
        let mut top = TopDict {
            charset: None,
            char_strings: 0,
            private: None,
            ros: false,
            fd_array: None,
            fd_select: None,
        };
        for entry in DictEntries::new(data) {
            match entry.op {
                OP_CHARSET => top.charset = dict_offset(&entry).ok()?,
                OP_CHAR_STRINGS => top.char_strings = dict_offset(&entry).ok()??,
                OP_PRIVATE => top.private = dict_range(&entry).ok()?,
                OP_ROS => top.ros = true,
                OP_FD_ARRAY => top.fd_array = dict_offset(&entry).ok()?,
                OP_FD_SELECT => top.fd_select = dict_offset(&entry).ok()?,
                _ => {}
            }
        }
        Some(top)
    }
}

/// The Private DICT's `Subrs` offset (relative to the Private DICT; later entries win).
struct PrivateDict {
    local_subrs: Option<usize>,
}

impl PrivateDict {
    /// `None` past `DICT_LEN_MAX` or where the `Subrs` offset cannot be followed.
    fn parse(data: &[u8]) -> Option<PrivateDict> {
        if data.len() > DICT_LEN_MAX {
            return None;
        }
        let mut local_subrs = None;
        for entry in DictEntries::new(data) {
            if entry.op == OP_LOCAL_SUBRS {
                local_subrs = dict_offset(&entry).ok()?;
            }
        }
        Some(PrivateDict { local_subrs })
    }
}

/// `parse_charset`: format 0 (n − 1 SIDs) or ranges covering exactly n − 1 glyphs.
fn parse_charset(data: &[u8], at: usize, glyphs: u16) -> Option<Charset<'_>> {
    let body = at.checked_add(1)?;
    let wanted = glyphs.checked_sub(1)?;
    match *data.get(at)? {
        0 => {
            let end = body.checked_add(usize::from(wanted).checked_mul(2)?)?;
            Some(Charset::Format0(data.get(body..end)?))
        }
        format @ (1 | 2) => {
            let wide = format == 2;
            let step = if wide { 4 } else { 3 };
            let (mut left_total, mut count, mut pos) = (wanted, 0usize, body);
            while left_total > 0 {
                let left = if wide {
                    u16_at(data, pos.checked_add(2)?)?.checked_add(1)?
                } else {
                    u16::from(*data.get(pos.checked_add(2)?)?) + 1
                };
                left_total = left_total.checked_sub(left)?;
                count += 1;
                pos = pos.checked_add(step)?;
            }
            let end = body.checked_add(count.checked_mul(step)?)?;
            Some(Charset::Ranges {
                data: data.get(body..end)?,
                wide,
            })
        }
        _ => None,
    }
}

impl FdSelect<'_> {
    /// The Font DICT index of `glyph` and the ranges walked to find it.
    fn font_dict_index(&self, glyph: u16) -> (Option<u8>, u64) {
        match self {
            FdSelect::Format0(array) => (array.get(usize::from(glyph)).copied(), 1),
            FdSelect::Format3(data) => {
                let mut walked = 1u64;
                let found = (|| {
                    let ranges = u16_at(data, 0)?;
                    if ranges == 0 {
                        return None;
                    }
                    // The sentinel GID closes the last range (ttf-parser counts it as one more).
                    let bound = ranges.checked_add(1)?;
                    let mut prev_first = u16_at(data, 2)?;
                    let mut prev_index = *data.get(4)?;
                    let mut pos = 5usize;
                    for _ in 1..bound {
                        walked += 1;
                        let first = u16_at(data, pos)?;
                        if (prev_first..first).contains(&glyph) {
                            return Some(prev_index);
                        }
                        prev_index = *data.get(pos.checked_add(2)?)?;
                        prev_first = first;
                        pos = pos.checked_add(3)?;
                    }
                    None
                })();
                (found, walked)
            }
        }
    }
}
