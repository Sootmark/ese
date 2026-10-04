//! B+-trees, walked from the root in key order one leaf entry at a time.
//!
//! Damage never stops the walk: a page that can't be read, belongs to
//! another tree, is a space tree page, or was already visited (a cycle, or a
//! page claimed twice) is reported and skipped, and so is an entry that
//! doesn't fit its page. Every page is visited at most once per walk, so a
//! walk ends, and memory stays proportional to the file. Defunct entries
//! (deleted, not yet cleaned up) are skipped, as ESE skips them.

use std::collections::HashSet;

use crate::page::{Node, Page};
use crate::pager::Pages;

/// A leaf entry, and the page holding it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry<'a> {
    pub(crate) page: u32,
    pub(crate) node: Node<'a>,
}

/// A page being walked: its entries are tags 1 and up.
struct Frame<'a> {
    page: Page<'a>,
    next_tag: usize,
    tag_count: usize,
}

/// A walk through one B+-tree, in key order.
pub(crate) struct Cursor<'a> {
    pages: &'a Pages<'a>,
    /// The tree's object identifier, from its root page; every page of the
    /// tree carries it.
    object_id: Option<u32>,
    stack: Vec<Frame<'a>>,
    visited: HashSet<u32>,
    /// What went wrong on the way, in the order met.
    pub(crate) problems: Vec<String>,
}

impl<'a> Cursor<'a> {
    pub(crate) fn new(pages: &'a Pages<'a>, root: u32) -> Self {
        let mut cursor = Self {
            pages,
            object_id: None,
            stack: Vec::new(),
            visited: HashSet::new(),
            problems: Vec::new(),
        };
        cursor.descend(root);
        cursor
    }

    /// Push page `number`, if it can be read as a page of this tree.
    fn descend(&mut self, number: u32) {
        match self.frame(number) {
            Ok(frame) => self.stack.push(frame),
            Err(problem) => self.problems.push(problem),
        }
    }

    fn frame(&mut self, number: u32) -> Result<Frame<'a>, String> {
        if !self.visited.insert(number) {
            return Err(format!(
                "page {number} reached twice in one tree (a cycle?)"
            ));
        }
        let page = self.pages.get(number)?;
        if page.is_blank() {
            return Err(format!("page {number} is blank (never written)"));
        }
        if let Some(problem) = self.pages.verify(&page) {
            self.problems.push(problem);
        }
        self.check_membership(&page)?;
        let flags = page.flags();
        if flags.is_space_tree() {
            return Err(format!(
                "page {number} is a space tree page, not a data page"
            ));
        }
        if flags.is_empty() {
            return Err(format!("page {number} is marked empty"));
        }
        Ok(Frame {
            page,
            next_tag: 1,
            tag_count: page.tag_count()?,
        })
    }

    /// Check that `page` belongs to this tree: the root sets the tree's
    /// object identifier, every other page must carry it.
    fn check_membership(&mut self, page: &Page) -> Result<(), String> {
        let object_id = page.fdp_object_id();
        match self.object_id {
            None => {
                self.object_id = Some(object_id);
                if !page.flags().is_root() {
                    self.problems.push(format!(
                        "page {}: a tree's root page without the root flag",
                        page.number
                    ));
                }
                Ok(())
            }
            Some(expected) if expected != object_id => Err(format!(
                "page {} belongs to object {object_id}, not to this tree's {expected}",
                page.number
            )),
            Some(_) => Ok(()),
        }
    }

    /// The next leaf entry in key order, or `None` at the end of the tree.
    pub(crate) fn next_entry(&mut self) -> Option<Entry<'a>> {
        while let Some(frame) = self.stack.last_mut() {
            if frame.next_tag >= frame.tag_count {
                self.stack.pop();
                continue;
            }
            let index = frame.next_tag;
            frame.next_tag += 1;
            let page = frame.page;
            let node = match page.node(index) {
                Ok(node) => node,
                Err(problem) => {
                    self.problems.push(problem);
                    continue;
                }
            };
            if node.flags.is_defunct() {
                continue;
            }
            if page.flags().is_leaf() {
                return Some(Entry {
                    page: page.number,
                    node,
                });
            }
            match page.child(&node) {
                Ok(child) => self.descend(child),
                Err(problem) => self.problems.push(problem),
            }
        }
        None
    }
}

impl<'a> Iterator for Cursor<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        self.next_entry()
    }
}
