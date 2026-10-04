//! Whether qpdf's overlay join leaves a page's content meaning what its parts meant (review-T5
//! H1). `content::qpdf_join` writes a `\n` after every part that does not end in one; that
//! newline only means nothing when the part boundary is neutral. The rule is the walker's own
//! (`content::joins`, review-final LOW-1): a streaming scan of the parts' lexical state at each
//! boundary, with no op cap, so a page the strict lexer refuses (over 250,000 ops, trailing
//! operands, unknown operators) is still judged by its boundaries alone (review-final MEDIUM-2).

use crate::pdf_engine::text_edit::content::first_unsafe_boundary;

/// Positions of the `\n` bytes `qpdf_join(parts)` inserts, and the joined length (mirrors
/// `content::qpdf_join`).
fn join_points(parts: &[&[u8]]) -> (Vec<usize>, usize) {
    let mut points = Vec::new();
    let mut len = 0usize;
    let mut need_newline = false;
    for part in parts {
        if need_newline {
            points.push(len);
            len = len.saturating_add(1);
        }
        let last = match part.last() {
            Some(c) => Some(*c),
            None if need_newline => Some(b'\n'),
            None => None,
        };
        len = len.saturating_add(part.len());
        need_newline = last != Some(b'\n');
    }
    (points, len)
}

/// Whether `joined` (= `content::qpdf_join(parts)`) reads as the parts read one after the other:
/// it is qpdf's join of `parts`, and every boundary between non-empty parts is neutral
/// (`content::joins`). `true` when the join inserts nothing.
pub(crate) fn join_is_neutral(parts: &[&[u8]], joined: &[u8]) -> bool {
    let (points, len) = join_points(parts);
    if points.is_empty() {
        return true;
    }
    if len != joined.len() || points.iter().any(|p| joined.get(*p) != Some(&b'\n')) {
        return false;
    }
    first_unsafe_boundary(parts).is_none()
}

#[cfg(test)]
mod tests {
    use super::join_is_neutral;
    use crate::pdf_engine::text_edit::content::qpdf_join;

    fn neutral(parts: &[&[u8]]) -> bool {
        join_is_neutral(parts, &qpdf_join(parts))
    }

    #[test]
    fn join_neutral_accepts_operator_and_whitespace_boundaries() {
        let ok: [&[&[u8]]; 7] = [
            &[
                b"BT /F1 12 Tf 72 720 Td (A) Tj ET",
                b"BT /F1 12 Tf 72 700 Td (B) Tj ET",
            ],
            &[b"q 1 0 0 1 0 0 cm", b"BT /F1 12 Tf (A) Tj ET Q"],
            &[b"Q", b"q"],
            &[b"ET ", b"BT ET"],
            &[b"0 g\n", b"1 g"],
            &[b"[(a) 10", b"(b)] TJ"],
            &[b"q\n% note\n", b"Q"],
        ];
        for parts in ok {
            assert!(neutral(parts), "{parts:?}");
        }
        assert!(neutral(&[]), "no parts");
        assert!(neutral(&[b"BT ET\n", b""]), "nothing inserted");
    }

    #[test]
    fn join_neutral_refuses_boundaries_inside_tokens_and_comments() {
        let refused: [&[&[u8]]; 11] = [
            // review-T5 R1: the comment ran on into the next part and hid it.
            &[
                b"BT /F1 12 Tf (Visible) Tj ET\n% note",
                b"BT /F1 12 Tf (Secret) Tj ET",
            ],
            &[b"BT ET % note", b"BT ET"],
            // review-T5: a string split between parts.
            &[b"BT /F1 12 Tf 72 700 Td (Hel", b"lo) Tj ET"],
            &[b"BT /F1 12 Tf 72 700 Td <48", b"65> Tj ET"],
            // Numbers and names that the plain concatenation reads as one token.
            &[b"1 0 0 1 0 1", b"0 cm"],
            &[b"BT /F", b"1 12 Tf ET"],
            &[b"/GS0", b"gs"],
            // Operators that would merge into another operator for pdf.js.
            &[b"0 0 m 10 10 l s", b"h"],
            &[b"[] 0 d", b"0 0 m"],
            // An inline image split inside its data.
            &[b"q BI /W 2 /H 1 /BPC 8 /CS /G ID \x01", b"\x02 EI Q"],
            // A boundary inside an unterminated string.
            &[b"BT (unterminated", b" Tj ET"],
        ];
        for parts in refused {
            assert!(!neutral(parts), "{parts:?}");
        }
        assert!(
            !join_is_neutral(&[b"Q", b"q"], b"Qq"),
            "a joined buffer that is not qpdf_join(parts)"
        );
    }
}
