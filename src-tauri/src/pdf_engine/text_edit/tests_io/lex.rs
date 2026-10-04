//! LEX-01…22 (+ scan modes): the byte-offset content tokenizer.

use crate::pdf_engine::text_edit::lexer::{
    check_arity, lex_content, scan_dict_at, scan_tokens, InlineProof, LexError, LexLimits, Op,
    Operand, Operator, ScanMode, Token,
};
use crate::pdf_engine::text_edit::limits;
use crate::pdf_engine::text_edit::testkit::pdf::zlib;
use std::sync::atomic::AtomicBool;

fn lex(src: &[u8]) -> Result<Vec<Op>, LexError> {
    lex_content(src, &LexLimits::page(), None)
}

fn operators(src: &[u8]) -> Vec<Operator> {
    lex(src).unwrap().iter().map(|o| o.operator).collect()
}

fn malformed(src: &[u8]) -> bool {
    matches!(lex(src), Err(LexError::Malformed { .. }))
}

fn str_operand(op: &Op) -> Vec<u8> {
    op.operands[0].as_str_bytes().unwrap().to_vec()
}

#[test]
fn lex01_whitespace_and_comments() {
    let src = b"\x00\t q %comment ( [ <<\r\n1 0 0 1 5 5 %mid\n cm\x0cQ";
    let ops = lex(src).unwrap();
    assert_eq!(
        ops.iter().map(|o| o.operator).collect::<Vec<_>>(),
        [Operator::q, Operator::cm, Operator::Q],
        "LEX-01"
    );
    let cm = &ops[1];
    assert_eq!(
        &src[cm.span.clone()],
        b"1 0 0 1 5 5 %mid\n cm",
        "LEX-01 comment inside the op span"
    );
    assert_eq!(&src[cm.op_span.clone()], b"cm");
    assert_eq!(ops[0].span, ops[0].op_span, "LEX-01 operand-less op span");
}

#[test]
fn lex02_literal_strings() {
    let ops =
        lex(b"(a(b)c) Tj (\\n\\r\\t\\b\\f\\(\\)\\\\) Tj (\\101\\060\\0617\\7\\q) Tj").unwrap();
    assert_eq!(str_operand(&ops[0]), b"a(b)c", "LEX-02 nesting");
    assert_eq!(
        str_operand(&ops[1]),
        b"\n\r\t\x08\x0c()\\",
        "LEX-02 escapes"
    );
    assert_eq!(
        str_operand(&ops[2]),
        b"A017\x07q",
        "LEX-02 octal + unknown escape"
    );
    let ops = lex(b"(line\\\ncont) Tj (a\r\nb) Tj (a\rb) Tj (a\\\r\nb) Tj").unwrap();
    let got: Vec<Vec<u8>> = ops.iter().map(str_operand).collect();
    assert_eq!(
        got,
        [
            b"linecont".to_vec(),
            b"a\nb".to_vec(),
            b"a\nb".to_vec(),
            b"ab".to_vec()
        ],
        "LEX-02 EOL rules"
    );
    assert!(malformed(b"(abc Tj"), "LEX-02 unterminated");
    let deep_ok = format!("{}{} Tj", "(".repeat(100), ")".repeat(100));
    assert!(lex(deep_ok.as_bytes()).is_ok(), "LEX-02 100 nested parens");
    let deep = format!("{}{} Tj", "(".repeat(101), ")".repeat(101));
    assert_eq!(
        lex(deep.as_bytes()),
        Err(LexError::TooComplex {
            what: "string nesting"
        })
    );
}

#[test]
fn lex03_hex_strings() {
    let ops = lex(b"<48 65 6C6c\n6F> Tj <414> Tj").unwrap();
    assert_eq!(str_operand(&ops[0]), b"Hello", "LEX-03");
    assert_eq!(
        str_operand(&ops[1]),
        [0x41, 0x40],
        "LEX-03 odd nibble padded with 0"
    );
    assert!(matches!(ops[0].operands[0], Operand::Str { hex: true, .. }));
    assert!(malformed(b"<4G> Tj"), "LEX-03 non-hex");
    assert!(malformed(b"<41 Tj"), "LEX-03 unterminated");
}

#[test]
fn lex04_names() {
    let ops = lex(b"/A#20B gs /#41 gs /a#zz gs / gs").unwrap();
    let names: Vec<&[u8]> = ops
        .iter()
        .map(|o| o.operands[0].as_name().unwrap())
        .collect();
    assert_eq!(names, [&b"A B"[..], b"A", b"a#zz", b""], "LEX-04");
}

#[test]
fn lex05_numbers_ok_and_precision() {
    let ops = lex(b".5 -.002 4. +3 re 0.123456789 g 1000000000 w").unwrap();
    let vals: Vec<f64> = ops[0]
        .operands
        .iter()
        .map(|o| o.as_number().unwrap())
        .collect();
    assert_eq!(vals, [0.5, -0.002, 4.0, 3.0], "LEX-05");
    assert_eq!(
        ops[1].operands[0].as_number(),
        Some(0.123456789),
        "LEX-05 f64 from exact digits"
    );
    assert_eq!(ops[2].operands[0].as_number(), Some(1e9));
    assert!(malformed(b"1000000001 w"), "LEX-05 |x| > 1e9");
}

#[test]
fn lex06_numbers_bad() {
    for src in [
        &b"--3 g"[..],
        b"1e5 g",
        b". g",
        b"5- g",
        b"+ g",
        b"1.2.3 g",
        b"0x10 g",
    ] {
        assert!(malformed(src), "LEX-06 {}", String::from_utf8_lossy(src));
    }
}

#[test]
fn lex07_nesting_to_the_limit() {
    let ok = format!("/P << /K {}{} >> BDC", "[".repeat(31), "]".repeat(31));
    assert!(lex(ok.as_bytes()).is_ok(), "LEX-07 32 levels");
    let over = format!("/P << /K {}{} >> BDC", "[".repeat(32), "]".repeat(32));
    assert_eq!(
        lex(over.as_bytes()),
        Err(LexError::TooComplex {
            what: "array or dictionary nesting"
        }),
        "LEX-07 +1"
    );
}

#[test]
fn lex08_array_items_to_the_limit() {
    let make = |n: usize| format!("[{}] TJ", "1 ".repeat(n));
    assert!(lex(make(65_536).as_bytes()).is_ok(), "LEX-08");
    assert_eq!(
        lex(make(65_537).as_bytes()),
        Err(LexError::TooComplex {
            what: "array items"
        }),
        "LEX-08 +1"
    );
    let operands = format!("{}n", "1 ".repeat(65_537));
    assert_eq!(
        lex(operands.as_bytes()),
        Err(LexError::TooComplex { what: "operands" })
    );
}

#[test]
fn lex09_d0_d1_are_single_ops() {
    assert_eq!(
        operators(b"0 0 d0 0 0 0 0 1 1 d1"),
        [Operator::d0, Operator::d1],
        "LEX-09"
    );
    assert_eq!(operators(b"[3 2] 0 d"), [Operator::d]);
    let all = b"b B b* B* BDC BI BMC BT BX c cm CS cs d d0 d1 Do DP EI EMC ET EX f F f* G g gs h i ID j J K k l m M MP n q Q re RG rg ri s S SC sc SCN scn sh T* Tc Td TD Tf Tj TJ TL Tm Tr Ts Tw Tz v w W W* y ' \"";
    let names: Vec<&[u8]> = all.split(|c| *c == b' ').collect();
    assert_eq!(names.len(), 73, "the 73 PDF 2.0 operators");
    for n in names {
        let op = Operator::from_token(n);
        assert_ne!(op, Operator::Unknown, "{}", String::from_utf8_lossy(n));
        assert_eq!(op.as_str().as_bytes(), n, "as_str inverts from_token");
    }
    assert_eq!(Operator::from_token(b"Tx"), Operator::Unknown);
}

#[test]
fn lex10_quote_ops_and_arity() {
    let ops = lex(b"(a) ' 1 2 (b) \" T*").unwrap();
    assert_eq!(
        ops.iter().map(|o| o.operator).collect::<Vec<_>>(),
        [Operator::Quote, Operator::DoubleQuote, Operator::TStar]
    );
    assert_eq!(ops[1].operands.len(), 3, "LEX-10");
    for bad in [
        &b"1 2 (b) '"[..],
        b"1 2 Tf",
        b"/F1 Tf",
        b"(a) (b) Tj",
        b"[[1]] TJ",
        b"1 2 3 cm",
        b"/A BDC",
        b"1 g 2 G 3 k",
    ] {
        assert!(
            malformed(bad),
            "LEX-10 arity {}",
            String::from_utf8_lossy(bad)
        );
    }
    assert!(lex(b"/P <</MCID 0>> BDC EMC /Pattern cs /P1 scn 0.5 /P1 scn 1 0 0 sc").is_ok());
}

#[test]
fn lex11_unknown_ops_and_compat() {
    assert_eq!(
        lex(b"q foo Q"),
        Err(LexError::Malformed {
            at: 2,
            what: "unknown operator"
        }),
        "LEX-11"
    );
    let ops = lex(b"BX foo 1 bar BX baz EX EX q").unwrap();
    let flags: Vec<(Operator, bool)> = ops.iter().map(|o| (o.operator, o.in_compat)).collect();
    assert_eq!(
        flags,
        [
            (Operator::BX, false),
            (Operator::Unknown, true),
            (Operator::Unknown, true),
            (Operator::BX, true),
            (Operator::Unknown, true),
            (Operator::EX, true),
            (Operator::EX, true),
            (Operator::q, false)
        ],
        "LEX-11"
    );
    assert_eq!(ops[2].operands.len(), 1);
}

fn image(src: &[u8]) -> (Vec<Op>, usize) {
    let ops = lex(src).unwrap();
    let i = ops
        .iter()
        .position(|o| o.operator == Operator::BI)
        .expect("BI op");
    (ops, i)
}

#[test]
fn lex12_inline_image_length_key() {
    let src = b"q BI /W 2 /H 2 /CS /G /BPC 8 /L 4 ID a EI EI Q";
    let (ops, i) = image(src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(img.proof, InlineProof::LengthKey, "LEX-12");
    assert_eq!(&src[img.data.clone()], b"a EI");
    assert_eq!(
        &src[ops[i].span.clone()],
        b"BI /W 2 /H 2 /CS /G /BPC 8 /L 4 ID a EI EI"
    );
    assert_eq!(ops[i + 1].operator, Operator::Q);
    assert!(!ops[i + 1].after_unproven_inline_image);
}

#[test]
fn lex13_inline_image_unfiltered_size() {
    let src = b"BI /W 14 /H 1 /CS /G /BPC 8 ID xx EI yy EI zz\nEI Q";
    let (ops, i) = image(src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(img.proof, InlineProof::UnfilteredSize, "LEX-13");
    assert_eq!(&src[img.data.clone()], b"xx EI yy EI zz");
    let mask = b"BI /IM true /W 9 /H 2 ID \x00\x01\x02\x03 EI Q";
    let (ops, i) = image(mask);
    assert_eq!(
        ops[i].inline_image.as_ref().unwrap().proof,
        InlineProof::UnfilteredSize,
        "LEX-13 /IM: 2 bytes x 2 rows"
    );
}

#[test]
fn lex14_inline_image_flate_end() {
    let payload = zlib(b"EI EI EI raw pixels");
    let mut src = b"BI /W 19 /H 1 /CS /G /BPC 8 /F /Fl ID ".to_vec();
    src.extend_from_slice(&payload);
    src.extend_from_slice(b"\nEI\nQ");
    let (ops, i) = image(&src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(img.proof, InlineProof::FlateEnd, "LEX-14");
    assert_eq!(
        &src[img.data.clone()],
        &payload[..],
        "LEX-14 data incl. Adler-32"
    );
}

#[test]
fn lex15_inline_image_ascii_ends() {
    let src = b"BI /W 2 /H 1 /CS /G /BPC 8 /F /AHx ID 0A 0B> EI Q";
    let (ops, i) = image(src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(
        (img.proof.clone(), &src[img.data.clone()]),
        (InlineProof::AsciiEnd, &b"0A 0B>"[..]),
        "LEX-15 AHx"
    );
    let src = b"BI /W 4 /H 1 /CS /G /BPC 8 /F [/A85] ID 87cURD]i~> EI Q";
    let (ops, i) = image(src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(
        (img.proof.clone(), &src[img.data.clone()]),
        (InlineProof::AsciiEnd, &b"87cURD]i~>"[..]),
        "LEX-15 A85"
    );
}

#[test]
fn lex16_inline_image_dct_heuristic_marks_followers() {
    let src = b"BI /W 2 /H 2 /CS /RGB /BPC 8 /F /DCT ID \xff\xd8 EI \x01\x02 junk\xff\xd9\nEI\nQ q 1 0 0 1 0 0 cm Q";
    let (ops, i) = image(src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(img.proof, InlineProof::Heuristic, "LEX-16");
    assert_eq!(&src[img.data.clone()], b"\xff\xd8 EI \x01\x02 junk\xff\xd9");
    assert!(!ops[i].after_unproven_inline_image);
    assert!(
        ops[i + 1..].iter().all(|o| o.after_unproven_inline_image),
        "LEX-16 every later op is marked"
    );
    assert_eq!(ops.len() - i - 1, 4);
}

#[test]
fn lex17_inline_image_without_candidate_and_crlf() {
    assert!(
        malformed(b"BI /W 2 /H 2 /F /DCT ID \xff\xd8\x00\x01\x02"),
        "LEX-17 no EI"
    );
    assert!(
        malformed(b"BI /W 2 /H 2 /F /DCT ID \xff\xd8 EIQ"),
        "LEX-17 EI not delimited"
    );
    let src = b"BI /W 3 /H 1 /CS /G /BPC 8 /L 3 ID\r\nabc EI Q";
    let (ops, i) = image(src);
    let img = ops[i].inline_image.as_ref().unwrap();
    assert_eq!(
        (&src[img.data.clone()], img.proof.clone()),
        (&b"abc"[..], InlineProof::LengthKey),
        "LEX-17 CR LF separator"
    );
    assert!(malformed(b"BI /W 1 ID"), "LEX-17 no separator");
    // At most INLINE_HEURISTIC_CANDIDATES_MAX whitespace-delimited `EI`s are lexed (each lex
    // reads a window): 255 failing candidates and the real end pass; one more is too complex.
    let candidates = |n: usize| {
        let mut src = b"BI /W 2 /H 2 /F /DCT ID \xff\xd8".to_vec();
        src.extend(b" EI \x01\x02".repeat(n));
        src.extend_from_slice(b"\nEI\nQ");
        src
    };
    let max = limits::INLINE_HEURISTIC_CANDIDATES_MAX;
    assert!(
        lex(&candidates(max - 1)).is_ok(),
        "LEX-17 candidates below the cap"
    );
    assert!(
        matches!(lex(&candidates(max)), Err(LexError::TooComplex { .. })),
        "LEX-17 candidate cap"
    );
}

#[test]
fn lex18_trailing_operands() {
    for src in [
        &b"q 1 0 0 1 0 0"[..],
        b"[1 2",
        b"<< /A 1",
        b"/F1 12 Tf /F1",
        b"BT (x) Tj ET 5",
    ] {
        assert_eq!(
            lex(src),
            Err(LexError::Malformed {
                at: src.len(),
                what: "trailing operands"
            }),
            "LEX-18 {src:?}"
        );
    }
}

#[test]
fn lex19_stray_delimiters() {
    for src in [
        &b"q ) Q"[..],
        b"q > Q",
        b"q ] Q",
        b"q >> Q",
        b"q { Q",
        b"q } Q",
        b"<< /A 1 >>",
        b"[1] q",
    ] {
        assert!(lex(src).is_err(), "LEX-19 {}", String::from_utf8_lossy(src));
    }
}

#[test]
fn lex20_garbage_mid_stream_is_an_error_never_a_prefix() {
    let src = b"BT /F1 12 Tf (ok) Tj 1 0 0 RG \x01\x02garbage ( ET";
    assert!(lex(src).is_err(), "LEX-20");
    // lopdf's decoder stops at the garbage and returns the prefix as Ok (why it is forbidden).
    let theirs = lopdf::content::Content::decode(src).map(|c| c.operations.len());
    println!("LEX-20 lopdf Content::decode on the probe: {theirs:?}");
    assert!(
        !matches!(theirs, Ok(5)),
        "LEX-20 probe: lopdf never sees the whole stream"
    );
}

#[test]
fn lex21_budgets_checked_while_lexing() {
    let limits = LexLimits {
        ops_max: 10,
        ..LexLimits::page()
    };
    let mut src = "q ".repeat(11).into_bytes();
    src.extend_from_slice(b") garbage (");
    let err = lex_content(&src, &limits, None).unwrap_err();
    assert_eq!(
        err,
        LexError::TooComplex { what: "operations" },
        "LEX-21 counter probe"
    );
    assert_eq!(
        err.page_reason(),
        crate::pdf_engine::text_edit::reasons::TextReason::PageTooComplex
    );
    assert_eq!(
        lex(b")").unwrap_err().page_reason(),
        crate::pdf_engine::text_edit::reasons::TextReason::MalformedContent
    );
    let many = "q Q ".repeat(3_000);
    let cancel = AtomicBool::new(true);
    assert_eq!(
        lex_content(many.as_bytes(), &LexLimits::page(), Some(&cancel)),
        Err(LexError::Cancelled),
        "LEX-21 cancel"
    );
    let few = "q Q ".repeat(1_000);
    assert!(
        lex_content(few.as_bytes(), &LexLimits::page(), Some(&cancel)).is_ok(),
        "cancel checked every 4,096 ops"
    );
}

/// Every valid sample of this file plus producer-like streams.
pub(crate) fn samples() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = [
        &b"\x00\t q %comment ( [ <<\r\n1 0 0 1 5 5 %mid\n cm\x0cQ"[..],
        b"BT /F1 12 Tf 72 720 Td [(H) -20 (ello) 250 (World)] TJ ET",
        b"q 1 0 0 1 0 0 cm 0.5 0.5 0.5 rg 10 10 100 50 re f Q /P <</MCID 0>> BDC BT /F1 1 Tf 12 0 0 12 72 700 Tm (x) Tj ET EMC",
        b"BT /F2 9 Tf <0041 0042> Tj T* (a) ' 1 2 (b) \" ET BX foo 1 bar EX",
        b"q BI /W 2 /H 2 /CS /G /BPC 8 /L 4 ID a EI EI Q",
        b"BI /W 14 /H 1 /CS /G /BPC 8 ID xx EI yy EI zz\nEI Q",
        b"BI /W 2 /H 2 /CS /RGB /BPC 8 /F /DCT ID \xff\xd8 EI \x01\x02 junk\xff\xd9\nEI\nQ q 1 0 0 1 0 0 cm Q",
        b"(a(b)c) Tj (\\n\\101) Tj <48 65> Tj /A#20B gs [3 2] 0 d 0 0 d0 0 0 0 0 1 1 d1 /Sh0 sh /Im1 Do",
        b"0 0 m 10 10 l 1 2 3 4 5 6 c 1 2 3 4 v 1 2 3 4 y h W n 2 w 1 J 0 j 4 M /GS0 gs 0 Tc 1 Tw 100 Tz 12 TL 0 Tr 2 Ts",
    ]
    .iter()
    .map(|s| s.to_vec())
    .collect();
    let mut flate = b"BI /W 19 /H 1 /CS /G /BPC 8 /F /Fl ID ".to_vec();
    flate.extend_from_slice(&zlib(b"EI EI EI raw pixels"));
    flate.extend_from_slice(b"\nEI\nQ");
    out.push(flate);
    out
}

#[test]
fn lex22_exact_spans_property() {
    for sample in samples() {
        for op in lex(&sample).unwrap().into_iter().filter(|o| !o.in_compat) {
            let sub = &sample[op.span.clone()];
            let again = lex(sub).unwrap_or_else(|e| {
                panic!("LEX-22 re-lex of {:?}: {e}", String::from_utf8_lossy(sub))
            });
            assert_eq!(again.len(), 1, "LEX-22 {:?}", String::from_utf8_lossy(sub));
            let (a, b) = (&again[0], &op);
            assert_eq!(a.operator, b.operator);
            assert_eq!(a.span, 0..sub.len(), "LEX-22 span covers the slice");
            assert_eq!(
                a.op_span.start + b.span.start..a.op_span.end + b.span.start,
                b.op_span
            );
            assert_eq!(a.operands.len(), b.operands.len());
            for (x, y) in a.operands.iter().zip(&b.operands) {
                assert_eq!(
                    x.span().start + b.span.start..x.span().end + b.span.start,
                    y.span().clone()
                );
            }
            if let (Some(x), Some(y)) = (&a.inline_image, &b.inline_image) {
                assert_eq!(
                    (
                        x.data.start + b.span.start..x.data.end + b.span.start,
                        &x.dict.len()
                    ),
                    (y.data.clone(), &y.dict.len())
                );
            }
            assert!(check_arity(a).is_ok() || a.operator == Operator::Unknown);
        }
    }
}

#[test]
fn lex23_scan_token_modes() {
    let toks = scan_tokens(
        b"<< /Root 1 0 R /Size 5 /ID [<AB> (x)] >>",
        ScanMode::Object,
        100,
    )
    .unwrap();
    assert!(
        toks.contains(&Token::Keyword {
            bytes: b"R".to_vec(),
            span: 13..14
        }),
        "Object mode keeps R as a keyword"
    );
    assert!(
        scan_tokens(b"{ 1 }", ScanMode::Object, 100).is_err(),
        "braces outside a procedure"
    );
    assert!(scan_tokens(b"1e-3 --5", ScanMode::Object, 100).is_err());
    let t1 = scan_tokens(
        b"/FontMatrix [0.001 0 0 0.001 0 0] readonly def { 1e-3 } bind",
        ScanMode::Type1Clear,
        100,
    )
    .unwrap();
    assert!(
        t1.iter()
            .any(|t| matches!(t, Token::Keyword { bytes, .. } if bytes == b"1e-3")),
        "Type1 lenient numbers"
    );
    let cmap = scan_tokens(
        b"/CIDInit /ProcSet findresource begin 1 begincodespacerange <00> <FF> endcodespacerange",
        ScanMode::CMap,
        100,
    )
    .unwrap();
    assert_eq!(cmap.len(), 9);
    assert!(
        scan_tokens(b"[ 1 2", ScanMode::CMap, 100).is_err(),
        "unbalanced"
    );
    assert_eq!(
        scan_tokens(b"1 2 3", ScanMode::Object, 2),
        Err(LexError::TooComplex { what: "tokens" })
    );
    let (dict, end) = scan_dict_at(b"trailer\n<< /Size 3 /Prev 10 >>\nstartxref", 7, 100).unwrap();
    assert_eq!((dict.len(), end), (6, 30));
    assert!(
        scan_dict_at(b" 12 0 obj", 0, 100).is_err(),
        "a dictionary is required"
    );
}
