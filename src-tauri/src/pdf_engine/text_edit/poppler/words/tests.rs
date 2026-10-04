//! G-TEXT unit tests: the neighbour join rules and the word classification (review-final
//! HIGH-1, review-verify HIGH-A).

use super::{classify, glyphs_in, joined_with_edit, Edge, FrameGlyph, Word};

fn w(text: &str, x0: f64, x1: f64) -> Word {
    Word {
        text: text.into(),
        x0,
        y0: 84.1,
        x1,
        y1: 96.4,
    }
}

/// review-final HIGH-1: a neighbour that does not match exactly counts only as joined with
/// the changed line's glyphs: longer text, its line, its outer edge, and the rest of the word
/// in the new box. The old `find` fallback (its text anywhere in a word at its place) is gone.
#[test]
fn a_neighbour_counts_as_joined_only_at_its_outer_edge_and_into_the_new_box() {
    let n = w("42", 105.34, 118.68);
    let new_box = [70.0, 80.0, 132.0, 100.0];
    let joined = |d: &Word, b: &[f64; 4]| joined_with_edit(d, &d.text, &n, "42", b, Edge::Whole);
    assert_eq!(
        joined(&w("world42", 90.0, 118.68), &new_box).as_deref(),
        Some("world")
    );
    assert_eq!(
        joined(&w("42ab", 105.34, 125.0), &new_box).as_deref(),
        Some("ab")
    );
    let refused = [
        ("moved", w("42", 105.82, 119.16)),
        ("moved and joined", w("x42", 95.0, 119.16)),
        ("relabelled 425 at its box", w("425", 105.34, 118.68)),
        ("relabelled 142 at its box", w("142", 105.34, 118.68)),
        ("inside a longer word", w("a42b", 95.0, 118.68)),
    ];
    for (what, d) in refused {
        assert_eq!(joined(&d, &new_box), None, "{what}");
    }
    let left_of_n = [70.0, 80.0, 100.0, 100.0];
    assert_eq!(
        joined(&w("42ab", 105.34, 125.0), &left_of_n),
        None,
        "join outside the edit"
    );
    let mut lower = w("x42", 95.0, 118.68);
    lower.y0 += 1.0;
    assert_eq!(joined(&lower, &new_box), None, "another line");
}

fn g(x0: f64, x1: f64, kept: Option<Option<&str>>) -> FrameGlyph {
    FrameGlyph {
        cx: (x0 + x1) / 2.0,
        cy: 90.0,
        x0,
        x1,
        y0: 84.1,
        y1: 96.4,
        kept: kept.map(|t| t.map(str::to_string)),
    }
}

/// "Hello" as the edited run's old glyphs (Helvetica 12 pt from x = 72).
fn hello() -> Vec<FrameGlyph> {
    let edges = [72.0, 80.67, 87.34, 90.0, 92.67, 99.34];
    edges.windows(2).map(|e| g(e[0], e[1], None)).collect()
}

/// review-verify HIGH-A: a word holding glyphs of runs no edit changes is a neighbour wherever
/// it lies; a word Poppler joined across the edit is split at its outer edges; a word that
/// cannot be split cleanly must come back whole.
#[test]
fn words_with_kept_glyphs_are_neighbours_and_joined_words_split_at_their_outer_edges() {
    // A 7 pt footnote marker whose centre lies within the old box's 2 pt pad.
    let marker = g(99.34, 103.23, Some(Some("1")));
    let (own, parts) = classify(&w("1", 99.34, 103.23), &[&marker], true);
    assert_eq!((own, parts.len(), parts[0].edge), (None, 1, Edge::Whole));
    // Nothing of ours in a word: the old box decides.
    let (own, parts) = classify(&w("Hello", 72.0, 99.34), &[], true);
    assert_eq!((own.as_deref(), parts.len()), (Some("Hello"), 0));
    assert_eq!(classify(&w("42", 140.0, 146.0), &[], false).1.len(), 1);
    // "Hello:" with a red ":": "Hello" is the run's, ":" is pinned at the word's x1.
    let edited = hello();
    let colon = g(99.34, 102.67, Some(Some(":")));
    let open = g(68.0, 72.0, Some(Some("(")));
    let mid = g(84.0, 85.0, Some(Some("x")));
    let semicolon = g(99.34, 102.67, Some(Some(";")));
    let unknown = g(99.34, 102.67, Some(None));
    let joined = |extra: &[&'static str]| -> Vec<&FrameGlyph> {
        let mut v: Vec<&FrameGlyph> = edited.iter().collect();
        for e in extra {
            v.extend(
                [&colon, &open, &mid, &semicolon, &unknown]
                    .into_iter()
                    .filter(|g| matches!(&g.kept, Some(t) if t.as_deref().unwrap_or("?") == *e)),
            );
        }
        v.sort_by(|a, b| a.cx.total_cmp(&b.cx));
        v
    };
    let (own, parts) = classify(&w("Hello:", 72.0, 102.67), &joined(&[":"]), true);
    assert_eq!(own.as_deref(), Some("Hello"));
    let p = &parts[0];
    assert_eq!(
        (
            parts.len(),
            p.edge,
            p.word.text.as_str(),
            p.word.x0,
            p.word.x1
        ),
        (1, Edge::End, ":", 99.34, 102.67)
    );
    // "(Hello:" kept at both ends.
    let (own, parts) = classify(&w("(Hello:", 68.0, 102.67), &joined(&["(", ":"]), true);
    assert_eq!(own.as_deref(), Some("Hello"));
    let edges: Vec<(Edge, &str, f64, f64)> = parts
        .iter()
        .map(|p| (p.edge, p.word.text.as_str(), p.word.x0, p.word.x1))
        .collect();
    assert_eq!(
        edges,
        [
            (Edge::Start, "(", 68.0, 72.0),
            (Edge::End, ":", 99.34, 102.67)
        ]
    );
    // Fail closed: a kept glyph between edited ones, a text Poppler does not read, a kept
    // glyph the font maps to nothing, or nothing of the run's left.
    let whole = [
        ("between", w("Hexllo", 72.0, 99.34), joined(&["x"])),
        ("other text", w("Hello:", 72.0, 102.67), joined(&[";"])),
        ("no text", w("Hello:", 72.0, 102.67), joined(&["?"])),
        ("nothing left", w(":", 72.0, 102.67), joined(&[":"])),
    ];
    for (what, word, inside) in whole {
        let (own, parts) = classify(&word, &inside, true);
        assert!(own.is_none(), "{what}");
        assert_eq!((parts.len(), parts[0].edge), (1, Edge::Whole), "{what}");
    }
}

/// review-verify HIGH-A: the part of a joined word holds only its outer edge (its inner edge is
/// our model's): it may come back alone there, or joined with the new glyphs.
#[test]
fn a_part_of_a_joined_word_is_found_only_at_its_outer_edge() {
    let n = w(":", 99.34, 102.67);
    let new_box = [70.0, 80.0, 100.0, 100.0];
    let at = |d: &Word, edge| joined_with_edit(d, &d.text, &n, ":", &new_box, edge);
    assert_eq!(at(&w(":", 99.3, 102.67), Edge::End).as_deref(), Some(""));
    assert_eq!(
        at(&w("Hellp:", 72.0, 102.67), Edge::End).as_deref(),
        Some("Hellp")
    );
    let refused = [
        ("moved", w(":", 99.0, 102.33)),
        ("relabelled", w(";", 99.34, 102.67)),
        ("wider at its outer edge", w(":", 99.34, 103.0)),
    ];
    for (what, d) in refused {
        assert_eq!(at(&d, Edge::End), None, "{what}");
    }
    assert_eq!(
        at(&w(":", 99.34, 102.67), Edge::Start).as_deref(),
        Some(""),
        "a start part keeps x0"
    );
    assert_eq!(
        at(&w(":", 99.34, 102.67), Edge::Whole),
        None,
        "a whole word is matched by its box, not here"
    );
}

/// No new false refusal (review-verify HIGH-A): a "." that the new, wider "e" covers is merged
/// by Poppler into the new word ("phrasee."); it is found inside it, but only when the new box
/// holds it and the word spans its box.
#[test]
fn a_covered_part_is_found_inside_the_new_word_that_spans_it() {
    let n = w(".", 379.55, 382.55);
    let new_box = [327.95, 80.0, 386.86, 100.0];
    let at = |d: &Word, b: &[f64; 4], edge| joined_with_edit(d, &d.text, &n, ".", b, edge);
    let merged = w("phrasee.", 348.24, 384.86);
    assert_eq!(at(&merged, &new_box, Edge::End).as_deref(), Some("phrasee"));
    let narrow = [327.95, 80.0, 381.0, 100.0];
    let refused = [
        (
            "not spanning its box",
            w("phrasee.", 380.0, 384.86),
            new_box,
        ),
        ("past the new box", w("phrasee.", 348.24, 390.0), new_box),
        ("not inside the new box", merged.clone(), narrow),
        ("not its text", w("phrasee", 348.24, 384.86), new_box),
    ];
    for (what, d, b) in refused {
        assert_eq!(at(&d, &b, Edge::End), None, "{what}");
    }
    assert_eq!(
        at(&merged, &new_box, Edge::Whole),
        None,
        "a whole word is not a part"
    );
}

/// A 9 pt stamp glyph whose centre falls inside a 26 pt title word is not the title's.
#[test]
fn a_glyph_belongs_to_a_word_only_across_its_middle() {
    let title = Word {
        text: "Quarterly".into(),
        x0: 33.75,
        y0: 34.11,
        x1: 136.38,
        y1: 60.69,
    };
    let mut letter = g(40.0, 50.0, Some(Some("Q")));
    (letter.cy, letter.y0, letter.y1) = (47.4, 34.1, 60.7);
    let mut stamp = g(72.0, 78.0, Some(Some("A")));
    (stamp.cy, stamp.y0, stamp.y1) = (37.7, 33.5, 41.9);
    let glyphs = [letter, stamp];
    let mut budget = 16;
    let inside = glyphs_in(&glyphs, &title, &mut budget).expect("within the budget");
    let texts: Vec<Option<String>> = inside.iter().map(|g| g.kept.clone().flatten()).collect();
    assert_eq!(texts, [Some("Q".to_string())]);
}
