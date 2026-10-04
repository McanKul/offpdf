//! The marked-content stack (`BMC`/`BDC` … `EMC`) as a persistent list. A push or a pop makes a
//! new stack that shares every entry below with the old one, so the stack each digest holds
//! costs O(1) per op whatever its depth: a page that reopens a deep stack with a new tag around
//! every show op keeps one node per op, not a copy of the whole stack (review T3 r2 MEDIUM-2).
//! Chains are at most `MARKED_DEPTH_MAX` long (the walker refuses a deeper push), so dropping one
//! recurses at most that deep.

use super::MarkedEntry;
use std::sync::Arc;

/// Innermost entry first. Cloning shares the whole stack.
#[derive(Debug, Clone, Default)]
pub struct MarkedStack(Option<Arc<MarkedNode>>);

/// One entry and the stack below it.
#[derive(Debug)]
pub struct MarkedNode {
    entry: MarkedEntry,
    below: MarkedStack,
    len: usize,
}

impl MarkedStack {
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |n| n.len)
    }

    pub fn push(&mut self, entry: MarkedEntry) {
        let below = std::mem::take(self);
        let len = below.len().saturating_add(1);
        *self = MarkedStack(Some(Arc::new(MarkedNode { entry, below, len })));
    }

    /// Removes the innermost entry (nothing when empty).
    pub fn pop(&mut self) {
        if let Some(top) = self.0.take() {
            *self = top.below.clone();
        }
    }

    /// The entries, innermost first.
    pub fn iter(&self) -> MarkedIter<'_> {
        MarkedIter(self.0.as_deref())
    }

    /// The very same stack (no entry compared): O(1).
    pub fn same(&self, other: &MarkedStack) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    /// Each node's address with its entry, innermost first (`approx_bytes` counts each node
    /// once).
    pub(crate) fn nodes(&self) -> impl Iterator<Item = (usize, &MarkedEntry)> {
        let mut cur = self.0.as_ref();
        std::iter::from_fn(move || {
            let node = cur?;
            cur = node.below.0.as_ref();
            Some((Arc::as_ptr(node) as usize, &node.entry))
        })
    }
}

/// Equal entries in the same order; stops at the first shared node.
impl PartialEq for MarkedStack {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let (mut a, mut b) = (self.0.as_ref(), other.0.as_ref());
        loop {
            match (a, b) {
                (None, None) => return true,
                (Some(x), Some(y)) => {
                    if Arc::ptr_eq(x, y) {
                        return true;
                    }
                    if x.entry != y.entry {
                        return false;
                    }
                    a = x.below.0.as_ref();
                    b = y.below.0.as_ref();
                }
                _ => return false,
            }
        }
    }
}

pub struct MarkedIter<'a>(Option<&'a MarkedNode>);

impl<'a> Iterator for MarkedIter<'a> {
    type Item = &'a MarkedEntry;

    fn next(&mut self) -> Option<&'a MarkedEntry> {
        let node = self.0?;
        self.0 = node.below.0.as_deref();
        Some(&node.entry)
    }
}
