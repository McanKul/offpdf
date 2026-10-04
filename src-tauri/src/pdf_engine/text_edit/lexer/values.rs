//! Operand values: nested arrays and dictionaries assembled iteratively from raw tokens, with
//! the nesting and item budgets of `LexLimits`.

use super::{malformed, LexError, LexLimits, Operand, Raw};

pub(super) enum Frame {
    Array {
        items: Vec<Operand>,
        start: usize,
    },
    Dict {
        entries: Vec<(Vec<u8>, Operand)>,
        key: Option<Vec<u8>>,
        start: usize,
    },
}

/// Collects operand values into nested arrays/dicts (iterative, depth- and size-capped).
pub(super) struct Builder {
    pub(super) frames: Vec<Frame>,
    nesting_max: usize,
    items_max: usize,
}

impl Builder {
    pub(super) fn new(lim: &LexLimits) -> Self {
        Builder {
            frames: Vec::new(),
            nesting_max: lim.nesting_max,
            items_max: lim.array_items_max,
        }
    }

    fn open(&mut self, frame: Frame) -> Result<(), LexError> {
        if self.frames.len() >= self.nesting_max {
            return Err(LexError::TooComplex {
                what: "array or dictionary nesting",
            });
        }
        self.frames.push(frame);
        Ok(())
    }

    /// Adds `v` to the innermost frame; returns it back when there is no open frame.
    fn add(&mut self, v: Operand) -> Result<Option<Operand>, LexError> {
        match self.frames.last_mut() {
            None => Ok(Some(v)),
            Some(Frame::Array { items, .. }) => {
                if items.len() >= self.items_max {
                    return Err(LexError::TooComplex {
                        what: "array items",
                    });
                }
                items.push(v);
                Ok(None)
            }
            Some(Frame::Dict { entries, key, .. }) => {
                match key.take() {
                    None => match v {
                        Operand::Name { bytes, .. } => *key = Some(bytes),
                        other => {
                            return malformed(other.span().start, "dictionary key is not a name")
                        }
                    },
                    Some(k) => {
                        if entries.len() >= self.items_max {
                            return Err(LexError::TooComplex {
                                what: "dictionary entries",
                            });
                        }
                        entries.push((k, v));
                    }
                }
                Ok(None)
            }
        }
    }

    fn close_array(&mut self, at: usize) -> Result<Operand, LexError> {
        match self.frames.pop() {
            Some(Frame::Array { mut items, start }) => {
                items.shrink_to_fit();
                Ok(Operand::Array {
                    items,
                    span: start..at + 1,
                })
            }
            _ => malformed(at, "stray ]"),
        }
    }

    fn close_dict(&mut self, at: usize) -> Result<Operand, LexError> {
        match self.frames.pop() {
            Some(Frame::Dict {
                mut entries,
                key: None,
                start,
            }) => {
                entries.shrink_to_fit();
                Ok(Operand::Dict {
                    entries,
                    span: start..at + 2,
                })
            }
            _ => malformed(at, "stray >>"),
        }
    }

    /// Feeds one raw token; returns a completed top-level value, if any.
    pub(super) fn feed(&mut self, raw: Raw) -> Result<Option<Operand>, LexError> {
        match raw {
            Raw::Operand(v) => self.add(v),
            Raw::ArrayOpen(p) => self
                .open(Frame::Array {
                    items: Vec::new(),
                    start: p,
                })
                .map(|_| None),
            Raw::DictOpen(p) => self
                .open(Frame::Dict {
                    entries: Vec::new(),
                    key: None,
                    start: p,
                })
                .map(|_| None),
            Raw::ArrayClose(p) => {
                let v = self.close_array(p)?;
                self.add(v)
            }
            Raw::DictClose(p) => {
                let v = self.close_dict(p)?;
                self.add(v)
            }
            Raw::Word(span) => malformed(span.start, "operator inside array or dictionary"),
            Raw::BraceOpen(p) | Raw::BraceClose(p) => malformed(p, "brace in content"),
        }
    }
}
