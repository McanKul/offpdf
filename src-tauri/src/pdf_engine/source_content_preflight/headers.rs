//! Linear-time discovery of every `N G obj` header candidate lopdf can parse.

use std::collections::BTreeMap;
use std::ops::Range;

pub(super) struct Headers<'a> {
    bytes: &'a [u8],
    next: usize,
    comment_runs: BTreeMap<usize, usize>,
    generations: BTreeMap<usize, Option<usize>>,
}

impl<'a> Headers<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            next: 0,
            comment_runs: BTreeMap::new(),
            generations: BTreeMap::new(),
        }
    }

    fn prune(&mut self, from: usize) {
        while self
            .comment_runs
            .first_key_value()
            .is_some_and(|(_, end)| *end <= from)
        {
            self.comment_runs.pop_first();
        }
        while self
            .generations
            .first_key_value()
            .is_some_and(|(at, _)| *at < from)
        {
            self.generations.pop_first();
        }
    }

    fn space(&mut self, at: usize) -> (usize, bool) {
        let mut index = at;
        while let Some(&byte) = self.bytes.get(index) {
            if byte == b'%' {
                return (self.comment_space(index), true);
            }
            if !is_whitespace(byte) {
                break;
            }
            index += 1;
        }
        (index, false)
    }

    fn known_comment_end(&self, at: usize) -> Option<usize> {
        self.comment_runs
            .range(..=at)
            .next_back()
            .map(|(_, end)| *end)
            .filter(|end| at < *end)
    }

    fn comment_space(&mut self, at: usize) -> usize {
        if let Some(end) = self.known_comment_end(at) {
            return end;
        }
        let mut index = at;
        let end = loop {
            match self.bytes.get(index) {
                Some(b'%') => {
                    if let Some(end) = self.known_comment_end(index) {
                        break end;
                    }
                    index = line_end(self.bytes, index);
                }
                Some(byte) if is_whitespace(*byte) => index += 1,
                _ => break index,
            }
        };
        let inner: Vec<usize> = self
            .comment_runs
            .range(at..end)
            .map(|(start, _)| *start)
            .collect();
        for start in inner {
            self.comment_runs.remove(&start);
        }
        self.comment_runs.insert(at, end);
        end
    }

    fn header_end(&mut self, run_end: usize) -> Option<usize> {
        let (generation_at, via_comment) = self.space(run_end);
        if let Some(found) = self.generations.get(&generation_at) {
            return *found;
        }
        let generation_end = digits_end(self.bytes, generation_at);
        let found = if generation_end == generation_at {
            None
        } else {
            let (object_at, _) = self.space(generation_end);
            let object_end = object_at.saturating_add(3);
            (self.bytes.get(object_at..object_end) == Some(&b"obj"[..])).then_some(object_end)
        };
        if via_comment {
            self.generations.insert(generation_at, found);
        }
        found
    }
}

impl Iterator for Headers<'_> {
    type Item = (Range<usize>, usize);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let relative = self
                .bytes
                .get(self.next..)?
                .iter()
                .position(u8::is_ascii_digit)?;
            let start = self.next + relative;
            let end = digits_end(self.bytes, start);
            self.next = end;
            self.prune(end);
            if let Some(value_at) = self.header_end(end) {
                return Some((start..end, value_at));
            }
        }
    }
}

fn is_whitespace(byte: u8) -> bool {
    matches!(byte, 0 | 9 | 10 | 12 | 13 | 32)
}

fn digits_end(bytes: &[u8], from: usize) -> usize {
    from + bytes.get(from..).map_or(0, |rest| {
        rest.iter().take_while(|c| c.is_ascii_digit()).count()
    })
}

fn line_end(bytes: &[u8], from: usize) -> usize {
    bytes
        .get(from..)
        .and_then(|rest| rest.iter().position(|c| matches!(c, b'\r' | b'\n')))
        .map_or(bytes.len(), |position| from + position)
}
