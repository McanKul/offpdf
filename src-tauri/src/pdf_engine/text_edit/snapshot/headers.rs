//! Every `N G obj` header lopdf 0.34's `_indirect_object` can parse, found in linear time.
//!
//! An xref offset may point at any digit, so every digit run in the file is a candidate first
//! number. After it lopdf needs `space G space obj`, where `space` is whitespace and `%` comments
//! (a comment runs to the next CR or LF). Probing each run naively rescans long comments, long
//! whitespace and long digit runs once per candidate, which is quadratic on crafted input
//! (review-T1 round 2, HIGH-1). Two memos make every byte cost O(1) probes:
//! - `runs`: for a `%` at `q`, the end `e` of lopdf's `space` from `q`. Any `%` in `[q, e)` either
//!   starts or sits inside a comment that ends at the same EOL, after which the walk is the same,
//!   so its `space` also ends at `e`. Stored intervals are therefore disjoint; one lookup answers
//!   every `%` inside them.
//! - `gens`: the result after the first `space`, keyed by where the generation number starts (it
//!   depends only on that position), so many candidates reaching the same generation number do
//!   not each rescan it and the `space` after it. Two candidates can only reach the same one when
//!   a comment hides the later candidate from the earlier one, so only probes whose first `space`
//!   went through a comment are stored (the others would cost an allocation per digit run).
//!
//! Probes never look behind the current digit run, so entries behind it are dropped and both memos
//! stay small.

use crate::pdf_engine::text_edit::lexer;
use std::collections::BTreeMap;
use std::ops::Range;

/// Iterator over `(first number run, offset after obj)` of every header candidate in a buffer.
pub(crate) struct Headers<'a> {
    b: &'a [u8],
    next: usize,
    runs: BTreeMap<usize, usize>,
    gens: BTreeMap<usize, Option<usize>>,
}

impl<'a> Headers<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Self {
        Headers {
            b,
            next: 0,
            runs: BTreeMap::new(),
            gens: BTreeMap::new(),
        }
    }

    /// Drops memo entries no probe starting at `from` or later can reach.
    fn prune(&mut self, from: usize) {
        while self
            .runs
            .first_key_value()
            .is_some_and(|(_, end)| *end <= from)
        {
            self.runs.pop_first();
        }
        while self
            .gens
            .first_key_value()
            .is_some_and(|(at, _)| *at < from)
        {
            self.gens.pop_first();
        }
    }

    /// lopdf's `space` from `at` (lenient: a comment may end at EOF), and whether it went
    /// through a comment.
    fn space(&mut self, at: usize) -> (usize, bool) {
        let mut i = at;
        while let Some(&c) = self.b.get(i) {
            if c == b'%' {
                return (self.comment_space(i), true);
            }
            if !lexer::is_whitespace(c) {
                break;
            }
            i += 1;
        }
        (i, false)
    }

    fn known(&self, q: usize) -> Option<usize> {
        self.runs
            .range(..=q)
            .next_back()
            .map(|(_, end)| *end)
            .filter(|end| q < *end)
    }

    /// End of lopdf's `space` from the `%` at `q`, memoised as the interval `[q, end)`.
    fn comment_space(&mut self, q: usize) -> usize {
        if let Some(end) = self.known(q) {
            return end;
        }
        let mut i = q;
        let end = loop {
            match self.b.get(i) {
                Some(b'%') => {
                    if let Some(end) = self.known(i) {
                        break end;
                    }
                    i = line_end(self.b, i);
                }
                Some(c) if lexer::is_whitespace(*c) => i += 1,
                _ => break i,
            }
        };
        // Intervals starting inside [q, end) end at `end` too: merge them into this one.
        let inner: Vec<usize> = self.runs.range(q..end).map(|(start, _)| *start).collect();
        for start in inner {
            self.runs.remove(&start);
        }
        self.runs.insert(q, end);
        end
    }

    /// After a first number ending at `run_end`: `space G space obj` → the offset after `obj`.
    fn header_end(&mut self, run_end: usize) -> Option<usize> {
        let (gen_at, via_comment) = self.space(run_end);
        if let Some(found) = self.gens.get(&gen_at) {
            return *found;
        }
        let gen_end = digits_end(self.b, gen_at);
        let found = if gen_end == gen_at {
            None
        } else {
            let (obj_at, _) = self.space(gen_end);
            let obj_end = obj_at.saturating_add(3);
            (self.b.get(obj_at..obj_end) == Some(&b"obj"[..])).then_some(obj_end)
        };
        if via_comment {
            self.gens.insert(gen_at, found);
        }
        found
    }
}

impl Iterator for Headers<'_> {
    type Item = (Range<usize>, usize);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let rel = self
                .b
                .get(self.next..)?
                .iter()
                .position(u8::is_ascii_digit)?;
            let start = self.next + rel;
            let end = digits_end(self.b, start);
            self.next = end;
            self.prune(end);
            if let Some(value_at) = self.header_end(end) {
                return Some((start..end, value_at));
            }
        }
    }
}

fn digits_end(b: &[u8], from: usize) -> usize {
    from + b
        .get(from..)
        .map_or(0, |r| r.iter().take_while(|c| c.is_ascii_digit()).count())
}

/// The CR or LF that ends the comment at `from` (or the end of the buffer).
fn line_end(b: &[u8], from: usize) -> usize {
    b.get(from..)
        .and_then(|r| r.iter().position(|c| matches!(c, b'\r' | b'\n')))
        .map_or(b.len(), |p| from + p)
}
