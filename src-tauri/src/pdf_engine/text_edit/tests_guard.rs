//! GUARD-01…03 (SPEC §B.21): the forbidden-API scan (comment- and string-aware), the lexer
//! fuzz, and no environment reads in production code.

use crate::pdf_engine::text_edit::lexer::{lex_content, scan_tokens, LexLimits, ScanMode};
use crate::pdf_engine::text_edit::tests_io::lex::samples;
use std::path::{Path, PathBuf};
use std::time::Instant;

const FORBIDDEN: &[&str] = &[
    ".unwrap()",
    ".expect(",
    "panic!",
    "unreachable!",
    "unimplemented!",
    "todo!",
    "Content::decode",
    "Content::encode",
    "string_to_bytes",
    "replace_text",
    "encode_text",
    "decompressed_content",
    "get_plain_content",
    "get_page_content",
    "Document::load",
    "load_mem",
    ".decompress()",
    "IncrementalDocument",
    ".save(",
    "save_to(",
];

/// First line of a `pdf_engine/*.rs` file that opts into the scan (T3's `source_content.rs`).
pub(crate) const SCAN_MARKER: &str = "//! offpdf:forbidden-api-scan";

/// The source with comments and the contents of string/char literals blanked (line breaks kept).
pub(crate) fn code_only(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let at = |i: usize| c.get(i).copied().unwrap_or('\0');
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let blank = |ch: char, out: &mut String| out.push(if ch == '\n' { '\n' } else { ' ' });
    while i < c.len() {
        let ch = c[i];
        let prev_ident = i > 0 && (at(i - 1).is_alphanumeric() || at(i - 1) == '_');
        if ch == '/' && at(i + 1) == '/' {
            while i < c.len() && c[i] != '\n' {
                i += 1;
            }
        } else if ch == '/' && at(i + 1) == '*' {
            let mut depth = 0usize;
            while i < c.len() {
                if c[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if c[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    blank(c[i], &mut out);
                    i += 1;
                }
            }
        } else if !prev_ident && (ch == 'r' || (ch == 'b' && at(i + 1) == 'r')) && {
            let s = if ch == 'b' { i + 2 } else { i + 1 };
            let mut j = s;
            while at(j) == '#' {
                j += 1;
            }
            at(j) == '"'
        } {
            let s = if ch == 'b' { i + 2 } else { i + 1 };
            let hashes = (s..).take_while(|j| at(*j) == '#').count();
            i = s + hashes + 1;
            out.push('"');
            while i < c.len() && !(c[i] == '"' && (1..=hashes).all(|k| at(i + k) == '#')) {
                blank(c[i], &mut out);
                i += 1;
            }
            out.push('"');
            i += 1 + hashes;
        } else if ch == '"' {
            out.push('"');
            i += 1;
            while i < c.len() && c[i] != '"' {
                if c[i] == '\\' {
                    blank(c[i], &mut out);
                    i += 1;
                }
                if i < c.len() {
                    blank(c[i], &mut out);
                    i += 1;
                }
            }
            out.push('"');
            i += 1;
        } else if ch == '\'' && at(i + 1) == '\\' {
            // escaped char literal ('\n', '\'', '\u{..}'): skip the escaped char, then find the quote
            let mut j = i + 3;
            while j < c.len() && j < i + 12 && c[j] != '\'' {
                j += 1;
            }
            i = j + 1;
            out.push_str("' '");
        } else if ch == '\'' && at(i + 2) == '\'' {
            i += 3; // 'x'
            out.push_str("' '");
        } else {
            // everything else, including lifetimes ('a) and labels
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// The part of a production file the scan reads: everything before the first line that
/// starts with `#[cfg(test)]` (unindented; an indented statement attribute does not cut).
fn production_part(src: &str) -> String {
    src.lines()
        .take_while(|l| !l.starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn violations(src: &str, patterns: &[&str]) -> Vec<(usize, String)> {
    code_only(&production_part(src))
        .lines()
        .enumerate()
        .flat_map(|(n, line)| {
            patterns
                .iter()
                .filter(move |p| line.contains(*p))
                .map(move |p| (n + 1, p.to_string()))
        })
        .collect()
}

fn text_edit_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pdf_engine/text_edit")
}

/// Every `.rs` under `text_edit/` except `testkit/`, `tests_*` files/directories and `tests.rs`
/// test-module files (`#[cfg(test)] mod tests;`), plus every `pdf_engine/*.rs` whose first line
/// is the scan marker.
fn scanned_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![text_edit_dir()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            if name == "testkit" || name == "tests.rs" || name.starts_with("tests_") {
                continue;
            }
            if p.is_dir() {
                stack.push(p);
            } else if name.ends_with(".rs") {
                out.push(p);
            }
        }
    }
    for entry in std::fs::read_dir(text_edit_dir().parent().unwrap()).unwrap() {
        let p = entry.unwrap().path();
        if p.extension().is_some_and(|e| e == "rs") {
            let src = std::fs::read_to_string(&p).unwrap();
            if src.lines().next() == Some(SCAN_MARKER) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn guard01_scanner_skips_comments_and_strings() {
    let clean = [
        "//! The docs may name Content::decode, .unwrap() and Document::load.\nfn f() {}",
        "/// Never call `get_page_content` here.\nfn f() {}",
        "fn f() { let s = \"a.unwrap() Document::load .save(\"; }",
        "fn f() { let r = r#\"panic!(\"x\") todo!()\"#; let b = br\"load_mem\"; }",
        "fn f() { /* outer /* .expect( */ IncrementalDocument */ }",
        "fn f() {}\n#[cfg(test)]\nmod tests { fn g() { x.unwrap(); Content::decode(b); } }",
        "fn f() { let c = '\"'; let d = '\\''; }",
        "fn f() { x.unwrap_or_default(); y.unwrap_or(0); z.expect_err_free(); }",
    ];
    for src in clean {
        assert_eq!(violations(src, FORBIDDEN), [], "GUARD-01 clean: {src}");
    }
    let dirty = [
        ("fn f() { let x = y.unwrap(); }", ".unwrap()"),
        ("fn f() { let c = '\"'; x.unwrap(); }", ".unwrap()"),
        (
            "fn f<'a>(x: &'a [u8]) { /* .unwrap() */ Content::decode(x); }",
            "Content::decode",
        ),
        ("fn f() { lopdf::Document::load(p); }", "Document::load"),
        (
            "fn f() {\n    #[cfg(test)]\n    y.expect(\"x\");\n}",
            ".expect(",
        ),
        ("fn f() { s.decompress(); }", ".decompress()"),
        ("fn f() { doc.save(p); }", ".save("),
        ("fn f() { panic!(\"no\"); }", "panic!"),
    ];
    for (src, what) in dirty {
        let v = violations(src, FORBIDDEN);
        assert!(
            v.iter().any(|(_, p)| p == what),
            "GUARD-01 must flag {what} in {src}: {v:?}"
        );
    }
}

#[test]
fn guard01_no_forbidden_apis() {
    let files = scanned_files();
    let names: Vec<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(text_edit_dir())
                .unwrap_or(p)
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    for must in [
        "snapshot.rs",
        "lexer.rs",
        "lexer/inline.rs",
        "decode.rs",
        "content.rs",
        "engines.rs",
        "reasons.rs",
        "limits.rs",
    ] {
        assert!(
            names.iter().any(|n| n == must),
            "GUARD-01 scans {must}: {names:?}"
        );
    }
    assert!(
        !names
            .iter()
            .any(|n| n.contains("testkit") || n.contains("tests")),
        "GUARD-01 skips test code"
    );
    let mut found = Vec::new();
    for (p, name) in files.iter().zip(&names) {
        let src = std::fs::read_to_string(p).unwrap();
        for (line, pattern) in violations(&src, FORBIDDEN) {
            found.push(format!("{name}:{line}: {pattern}"));
        }
    }
    assert!(
        found.is_empty(),
        "GUARD-01 forbidden APIs in production code:\n{}",
        found.join("\n")
    );
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const INSERTS: &[&[u8]] = &[
    b"(",
    b")",
    b"<",
    b">",
    b"<<",
    b">>",
    b"[",
    b"]",
    b"BI",
    b" BI ",
    b"ID ",
    b" EI ",
    b"q",
    b"Q",
    b"%",
    b"\\",
    b"99999999999999999999",
    b"1e308",
    b"-.",
    b"/",
    b"BX",
    b"EX",
    b"\r\n",
    b"\x00",
    b"\xff",
];

fn mutate(rng: &mut XorShift, base: &[u8]) -> Vec<u8> {
    let mut v = base.to_vec();
    for _ in 0..1 + rng.below(4) {
        let len = v.len();
        match rng.below(6) {
            0 if len > 0 => {
                let i = rng.below(len);
                v[i] ^= 1 << rng.below(8);
            }
            1 if len > 0 => v.truncate(rng.below(len)),
            2 => {
                let ins = INSERTS[rng.below(INSERTS.len())];
                let at = rng.below(len + 1);
                v.splice(at..at, ins.iter().copied());
            }
            3 if len > 1 => {
                let a = rng.below(len);
                let b = (a + 1 + rng.below(16)).min(len);
                v.drain(a..b);
            }
            4 if len > 1 => {
                let a = rng.below(len);
                let b = (a + 1 + rng.below(32)).min(len);
                let piece = v[a..b].to_vec();
                let at = rng.below(v.len() + 1);
                v.splice(at..at, piece);
            }
            _ => {
                let i = rng.below(len + 1);
                v.insert(i, rng.next() as u8);
            }
        }
    }
    v
}

#[test]
fn guard02_lexer_fuzz() {
    let seeds = samples();
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    let limits = LexLimits::page();
    let started = Instant::now();
    let mut errors = 0usize;
    for case in 0..10_000 {
        let base = &seeds[case % seeds.len()];
        let input = mutate(&mut rng, base);
        if lex_content(&input, &limits, None).is_err() {
            errors += 1;
        }
        for mode in [ScanMode::Object, ScanMode::CMap, ScanMode::Type1Clear] {
            let _ = scan_tokens(&input, mode, 10_000);
        }
    }
    let secs = started.elapsed().as_secs_f64();
    println!("GUARD-02 10,000 cases, {errors} lexed as errors, {secs:.2} s");
    assert!(
        errors > 0 && errors < 10_000,
        "GUARD-02 the fuzz reaches both outcomes"
    );
    assert!(secs < 20.0, "GUARD-02 < 2 s per 1,000 cases ({secs:.2} s)");
}

/// GUARD-03 rule: `env::var` only for the PATH lookup in `engines.rs`.
fn env_violations(name: &str, src: &str) -> Vec<usize> {
    let prod = production_part(src);
    let code = code_only(&prod);
    code.lines()
        .zip(prod.lines())
        .enumerate()
        .filter(|(_, (c, _))| c.contains("env::var"))
        .filter(|(_, (_, orig))| {
            !(name.ends_with("engines.rs") && orig.contains("env::var_os(\"PATH\")"))
        })
        .map(|(n, _)| n + 1)
        .collect()
}

#[test]
fn guard03_no_environment_reads_in_production() {
    assert_eq!(
        env_violations("x.rs", "fn f() { std::env::var(\"OFFPDF_X\"); }"),
        [1],
        "GUARD-03 self-test"
    );
    assert_eq!(
        env_violations("x.rs", "fn f() { std::env::var_os(\"PATH\"); }"),
        [1],
        "PATH only in engines.rs"
    );
    assert!(env_violations("engines.rs", "fn f() { std::env::var_os(\"PATH\"); }").is_empty());
    assert!(env_violations("x.rs", "// std::env::var(\"X\") in a comment\nfn f() {}").is_empty());
    assert!(env_violations(
        "x.rs",
        "fn f() {}\n#[cfg(test)]\nfn t() { std::env::var(\"X\"); }"
    )
    .is_empty());
    let mut found = Vec::new();
    for p in scanned_files() {
        let src = std::fs::read_to_string(&p).unwrap();
        let name = p.to_string_lossy().into_owned();
        for line in env_violations(&name, &src) {
            found.push(format!("{name}:{line}"));
        }
    }
    assert!(
        found.is_empty(),
        "GUARD-03 environment reads in production code:\n{}",
        found.join("\n")
    );
}
