// Copyright 2018 Google LLC
//
// Use of this source code is governed by an MIT-style
// license that can be found in the LICENSE file or at
// https://opensource.org/licenses/MIT.

//! A Vec-based container for a tree structure.

use std::num::NonZeroU32;
use std::ops::{Add, Sub};

use crate::parse::{Item, ItemBody};

#[derive(Debug, Eq, PartialEq, Copy, Clone, PartialOrd)]
pub(crate) struct TreeIndex(NonZeroU32);

impl TreeIndex {
    fn new(i: usize) -> Self {
        TreeIndex(
            NonZeroU32::new(u32::try_from(i).expect("too many Markdown tree nodes"))
                .expect("Markdown tree index must be nonzero"),
        )
    }

    pub fn get(self) -> usize {
        self.0.get() as usize
    }
}

impl Add<usize> for TreeIndex {
    type Output = TreeIndex;

    fn add(self, rhs: usize) -> Self {
        let inner = self
            .get()
            .checked_add(rhs)
            .expect("Markdown tree index overflow");
        TreeIndex::new(inner)
    }
}

impl Sub<usize> for TreeIndex {
    type Output = TreeIndex;

    fn sub(self, rhs: usize) -> Self {
        let inner = self
            .get()
            .checked_sub(rhs)
            .expect("Markdown tree index underflow");
        TreeIndex::new(inner)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Node<T> {
    pub child: Option<TreeIndex>,
    pub next: Option<TreeIndex>,
    pub item: T,
}

const STORAGE_CHUNK: usize = 8192;

/// Normal parsing retains Vec growth. Title parsing avoids moving a large live
/// arena when its capacity grows; numeric indices still identify the same slot.
#[derive(Clone)]
pub(crate) enum Storage<T> {
    Contiguous(Vec<T>),
    Chunked { chunks: Vec<Vec<T>>, len: usize },
}

impl<T> Storage<T> {
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self::Contiguous(Vec::with_capacity(capacity))
    }

    pub(crate) fn chunked() -> Self {
        Self::Chunked {
            chunks: Vec::new(),
            len: 0,
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Contiguous(values) => values.len(),
            Self::Chunked { len, .. } => *len,
        }
    }

    pub(crate) fn push(&mut self, value: T) {
        match self {
            Self::Contiguous(values) => values.push(value),
            Self::Chunked { chunks, len } => {
                if !matches!(chunks.last(), Some(last) if last.len() < STORAGE_CHUNK) {
                    chunks.push(Vec::with_capacity(STORAGE_CHUNK));
                }
                chunks.last_mut().unwrap().push(value);
                *len += 1;
            }
        }
    }

    pub(crate) fn pop(&mut self) -> Option<T> {
        match self {
            Self::Contiguous(values) => values.pop(),
            Self::Chunked { chunks, len } => {
                let last = chunks.last_mut()?;
                let value = last.pop()?;
                *len -= 1;
                if last.is_empty() {
                    chunks.pop();
                }
                Some(value)
            }
        }
    }
}

impl<T> std::ops::Index<usize> for Storage<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        match self {
            Self::Contiguous(values) => &values[index],
            Self::Chunked { chunks, .. } => &chunks[index / STORAGE_CHUNK][index % STORAGE_CHUNK],
        }
    }
}

impl<T> std::ops::IndexMut<usize> for Storage<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        match self {
            Self::Contiguous(values) => &mut values[index],
            Self::Chunked { chunks, .. } => {
                &mut chunks[index / STORAGE_CHUNK][index % STORAGE_CHUNK]
            }
        }
    }
}

/// A tree abstraction, intended for fast building as a preorder traversal.
#[derive(Clone)]
pub(crate) struct Tree<T> {
    nodes: Storage<Node<T>>,
    spine: Vec<TreeIndex>, // indices of nodes on path to current node
    cur: Option<TreeIndex>,
}

impl<T: Default> Tree<T> {
    // Indices start at one, so we place a dummy value at index zero.
    // The alternative would be subtracting one from every TreeIndex
    // every time we convert it to usize to index our nodes.
    pub(crate) fn with_capacity(cap: usize) -> Tree<T> {
        Self::with_storage(Storage::with_capacity(cap))
    }

    pub(crate) fn chunked() -> Tree<T> {
        Self::with_storage(Storage::chunked())
    }

    fn with_storage(mut nodes: Storage<Node<T>>) -> Tree<T> {
        nodes.push(Node {
            child: None,
            next: None,
            item: <T as Default>::default(),
        });
        Tree {
            nodes,
            spine: Vec::new(),
            cur: None,
        }
    }

    /// Returns the index of the element currently in focus.
    pub(crate) fn cur(&self) -> Option<TreeIndex> {
        self.cur
    }

    /// Append one item to the current position in the tree.
    pub(crate) fn append(&mut self, item: T) -> TreeIndex {
        let ix = self.create_node(item);
        let this = Some(ix);

        if let Some(ix) = self.cur {
            self[ix].next = this;
        } else if let Some(&parent) = self.spine.last() {
            self[parent].child = this;
        }
        self.cur = this;
        ix
    }

    /// Create an isolated node.
    pub(crate) fn create_node(&mut self, item: T) -> TreeIndex {
        let this = TreeIndex::new(self.nodes.len());
        self.nodes.push(Node {
            child: None,
            next: None,
            item,
        });
        this
    }

    /// Push down one level, so that new items become children of the current node.
    /// The new focus index is returned.
    pub(crate) fn push(&mut self) -> TreeIndex {
        let cur_ix = self.cur.unwrap();
        self.spine.push(cur_ix);
        self.cur = self[cur_ix].child;
        cur_ix
    }

    /// Pop back up a level.
    pub(crate) fn pop(&mut self) -> Option<TreeIndex> {
        let ix = Some(self.spine.pop()?);
        self.cur = ix;
        ix
    }

    /// Remove the last node, as `pop` but removing it.
    pub(crate) fn remove_node(&mut self) -> Option<TreeIndex> {
        let ix = self.spine.pop()?;
        self.cur = Some(ix);
        self.nodes.pop()?;
        self[ix].child = None;
        Some(ix)
    }

    /// Look at the parent node.
    pub(crate) fn peek_up(&self) -> Option<TreeIndex> {
        self.spine.last().copied()
    }

    /// Look at grandparent node.
    pub(crate) fn peek_grandparent(&self) -> Option<TreeIndex> {
        if self.spine.len() >= 2 {
            Some(self.spine[self.spine.len() - 2])
        } else {
            None
        }
    }

    /// Returns true when there are no nodes other than the root node
    /// in the tree, false otherwise.
    pub(crate) fn is_empty(&self) -> bool {
        self.nodes.len() <= 1
    }

    /// Returns the length of the spine.
    pub(crate) fn spine_len(&self) -> usize {
        self.spine.len()
    }

    /// Resets the focus to the first node added to the tree, if it exists.
    pub(crate) fn reset(&mut self) {
        self.cur = if self.is_empty() {
            None
        } else {
            Some(TreeIndex::new(1))
        };
        self.spine.clear();
    }

    /// Walks the spine from a root node up to, but not including, the current node.
    pub(crate) fn walk_spine(&self) -> impl std::iter::DoubleEndedIterator<Item = &TreeIndex> {
        self.spine.iter()
    }

    /// Moves focus to the next sibling of the given node.
    pub(crate) fn next_sibling(&mut self, cur_ix: TreeIndex) -> Option<TreeIndex> {
        self.cur = self[cur_ix].next;
        self.cur
    }

    pub(crate) fn truncate_to_parent(&mut self, child_ix: TreeIndex) {
        let next = self[child_ix].next;
        self[child_ix].next = None;
        if let Some(cur) = self.cur {
            self[cur].next = next;
        } else if let Some(&parent) = self.spine.last() {
            self[parent].child = next;
        }
        if next.is_some() {
            self.cur = next;
        }
    }
}

impl Tree<Item> {
    /// Truncates the preceding siblings to the given end position,
    /// and returns the new current node.
    pub(crate) fn truncate_siblings(&mut self, end_byte_ix: usize) {
        let parent_ix = self.peek_up().unwrap();
        let mut next_child_ix = self[parent_ix].child;
        let mut prev_child_ix = None;

        // drop or truncate children based on its range
        while let Some(child_ix) = next_child_ix {
            let child_end = self[child_ix].item.end;
            if child_end < end_byte_ix {
                // preserve this node, and go to the next
                prev_child_ix = Some(child_ix);
                next_child_ix = self[child_ix].next;
                continue;
            } else if child_end == end_byte_ix {
                // this will be the last node
                self[child_ix].next = None;
                // focus to the new last child (this node)
                self.cur = Some(child_ix);
            } else if self[child_ix].item.start == end_byte_ix {
                // check whether the previous character is a backslash
                let is_previous_char_backslash_escape = match self[child_ix].item.body {
                    ItemBody::Text { backslash_escaped } => backslash_escaped,
                    _ => false,
                };
                if is_previous_char_backslash_escape {
                    // rescue the backslash as a plain text content
                    let last_byte_ix = end_byte_ix - 1;
                    self[child_ix].item.start = last_byte_ix;
                    self[child_ix].item.end = end_byte_ix;
                    self.cur = Some(child_ix);
                } else if let Some(prev_child_ix) = prev_child_ix {
                    // the node will become empty. drop the node
                    // a preceding sibling exists
                    self[prev_child_ix].next = None;
                    self.cur = Some(prev_child_ix);
                } else {
                    // no preceding siblings. remove the node from the parent
                    self[parent_ix].child = None;
                    self.cur = None;
                }
            } else {
                debug_assert!(self[child_ix].item.start < end_byte_ix);
                debug_assert!(end_byte_ix < child_end);
                // truncate the node
                self[child_ix].item.end = end_byte_ix;
                self[child_ix].next = None;
                // focus to the new last child
                self.cur = Some(child_ix);
            }
            break;
        }
    }
}

impl<T> std::fmt::Debug for Tree<T>
where
    T: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn debug_tree<T>(
            tree: &Tree<T>,
            cur: TreeIndex,
            indent: usize,
            f: &mut std::fmt::Formatter<'_>,
        ) -> std::fmt::Result
        where
            T: std::fmt::Debug,
        {
            for _ in 0..indent {
                write!(f, "  ")?;
            }
            writeln!(f, "{:?}", &tree[cur].item)?;
            if let Some(child_ix) = tree[cur].child {
                debug_tree(tree, child_ix, indent + 1, f)?;
            }
            if let Some(next_ix) = tree[cur].next {
                debug_tree(tree, next_ix, indent, f)?;
            }
            Ok(())
        }

        if self.nodes.len() > 1 {
            let cur = TreeIndex(NonZeroU32::new(1).unwrap());
            debug_tree(self, cur, 0, f)
        } else {
            write!(f, "Empty tree")
        }
    }
}

impl<T> std::ops::Index<TreeIndex> for Tree<T> {
    type Output = Node<T>;

    fn index(&self, ix: TreeIndex) -> &Self::Output {
        self.nodes.index(ix.get())
    }
}

impl<T> std::ops::IndexMut<TreeIndex> for Tree<T> {
    fn index_mut(&mut self, ix: TreeIndex) -> &mut Node<T> {
        self.nodes.index_mut(ix.get())
    }
}

#[cfg(test)]
mod packed_index_tests {
    use super::*;

    #[test]
    fn packed_indices_preserve_checked_arithmetic_and_option_layout() {
        assert_eq!(std::mem::size_of::<Option<TreeIndex>>(), 4);
        let maximum = u32::MAX as usize;
        assert_eq!(TreeIndex::new(maximum).get(), maximum);
        assert_eq!((TreeIndex::new(maximum - 1) + 1).get(), maximum);
        assert_eq!((TreeIndex::new(maximum) - (maximum - 1)).get(), 1);
    }

    #[test]
    #[should_panic(expected = "too many Markdown tree nodes")]
    #[cfg(target_pointer_width = "64")]
    fn node_identifier_overflow_cannot_truncate() {
        let _ = TreeIndex::new(u32::MAX as usize) + 1;
    }

    #[test]
    #[should_panic(expected = "Markdown tree index must be nonzero")]
    fn zero_identifier_cannot_be_constructed() {
        let _ = TreeIndex::new(0);
    }

    #[test]
    #[should_panic(expected = "Markdown tree index must be nonzero")]
    fn arithmetic_result_cannot_be_zero() {
        let _ = TreeIndex::new(1) - 1;
    }

    #[test]
    #[should_panic(expected = "Markdown tree index overflow")]
    fn arithmetic_overflow_cannot_wrap() {
        let _ = TreeIndex::new(1) + usize::MAX;
    }

    #[test]
    #[should_panic(expected = "Markdown tree index underflow")]
    fn arithmetic_underflow_cannot_wrap() {
        let _ = TreeIndex::new(1) - 2;
    }
}

#[cfg(test)]
mod title_storage_tests {
    use super::*;

    #[test]
    fn chunk_boundaries_clone_pop_and_reinsert_preserve_owned_values() {
        let mut storage = Storage::chunked();
        assert_eq!(storage.pop(), None::<String>);
        let count = STORAGE_CHUNK * 2 + 3;
        for index in 0..count {
            storage.push(format!("value-{index}"));
        }
        for index in [
            0,
            STORAGE_CHUNK - 1,
            STORAGE_CHUNK,
            STORAGE_CHUNK * 2,
            count - 1,
        ] {
            assert_eq!(storage[index], format!("value-{index}"));
        }
        storage[STORAGE_CHUNK] = "changed".to_owned();
        let mut clone = storage.clone();
        clone[STORAGE_CHUNK] = "clone".to_owned();
        assert_eq!(storage[STORAGE_CHUNK], "changed");
        for index in (STORAGE_CHUNK..count).rev() {
            let expected = if index == STORAGE_CHUNK {
                "clone".to_owned()
            } else {
                format!("value-{index}")
            };
            assert_eq!(clone.pop(), Some(expected));
        }
        assert_eq!(clone.len(), STORAGE_CHUNK);
        clone.push("reinserted".to_owned());
        assert_eq!(clone[STORAGE_CHUNK], "reinserted");
        while clone.pop().is_some() {}
        assert_eq!(clone.len(), 0);
        clone.push("fresh".to_owned());
        assert_eq!(clone[0], "fresh");
        assert_eq!(storage.len(), count);
    }

    #[test]
    fn tree_sentinel_and_removed_child_slot_survive_chunk_boundary() {
        let mut tree = Tree::<usize>::chunked();
        assert!(tree.is_empty());
        for value in 1..=STORAGE_CHUNK {
            let index = tree.append(value);
            assert_eq!(index.get(), value);
            assert_eq!(tree[index].item, value);
        }
        let parent = tree.cur().unwrap();
        tree.push();
        let child = tree.append(99);
        assert_eq!(child.get(), STORAGE_CHUNK + 1);
        assert_eq!(tree.remove_node(), Some(parent));
        assert_eq!(tree[parent].child, None);
        assert_eq!(tree.nodes.len(), STORAGE_CHUNK + 1);
        tree.push();
        let replacement = tree.append(100);
        assert_eq!(replacement, child);
        tree.pop();
        let clone = tree.clone();
        tree[replacement].item = 101;
        assert_eq!(clone[replacement].item, 100);
        tree.reset();
        assert_eq!(tree.cur().unwrap().get(), 1);
        assert_eq!(tree.nodes[0].item, 0);
    }

    #[test]
    fn normal_storage_keeps_the_contiguous_vec_policy() {
        let mut storage = Storage::with_capacity(17);
        let mut original = Vec::with_capacity(17);
        for value in 0..100 {
            storage.push(value);
            original.push(value);
            let Storage::Contiguous(values) = &storage else {
                panic!("normal arena became chunked")
            };
            assert_eq!(values.capacity(), original.capacity());
            assert_eq!(values, &original);
        }
        let tree = Tree::<usize>::with_capacity(128);
        assert!(matches!(tree.nodes, Storage::Contiguous(_)));
    }
}
