//! Page registrar for the Settings app.
//!
//! Provides a lightweight registration system for settings pages using
//! function pointers (since `no_std` + `alloc` cannot use `Box<dyn Trait>`).
//! Each page declares its metadata via [`PageInfo`] and a build function
//! that constructs the UI panel when the page is selected.

use alloc::vec::Vec;
use libanyui_client as ui;

/// Metadata for a single settings page.
pub struct PageInfo {
    /// Unique identifier for this page (e.g. "general", "display").
    pub id: &'static str,
    /// Human-readable display name shown in the sidebar.
    pub name: &'static str,
    /// System icon name, or empty string for no icon.
    pub icon_name: &'static str,
    /// Category grouping (pages are sorted by category, then order).
    pub category: &'static str,
    /// Sort order within the category (lower values appear first).
    pub order: i32,
    /// Build function: given a parent ScrollView, constructs the page UI
    /// and returns the panel View ID.
    pub build_fn: fn(&ui::ScrollView) -> u32,
}

/// Collects and organises settings page registrations.
pub struct Registrar {
    pages: Vec<PageInfo>,
}

impl Registrar {
    /// Create an empty registrar.
    pub fn new() -> Self {
        Self { pages: Vec::new() }
    }

    /// Register a new settings page.
    pub fn register(&mut self, page: PageInfo) {
        self.pages.push(page);
    }

    /// Return a slice of all registered pages.
    pub fn pages(&self) -> &[PageInfo] {
        &self.pages
    }

    /// Return a mutable slice of all registered pages.
    pub fn pages_mut(&mut self) -> &mut [PageInfo] {
        &mut self.pages
    }

    /// Return the number of registered pages.
    pub fn len(&self) -> usize {
        self.pages.len()
    }

    /// Return `true` if no pages have been registered.
    pub fn is_empty(&self) -> bool {
        self.pages.is_empty()
    }

    /// Find a page by its unique identifier.
    pub fn find(&self, id: &str) -> Option<&PageInfo> {
        self.pages.iter().find(|p| p.id == id)
    }

    /// Find the index of a page by its unique identifier.
    pub fn find_index(&self, id: &str) -> Option<usize> {
        self.pages.iter().position(|p| p.id == id)
    }

    /// Sort all registered pages by (category, order) using insertion sort.
    ///
    /// Pages within the same category are ordered by their `order` field.
    /// Categories themselves are sorted lexicographically.
    pub fn sort(&mut self) {
        let n = self.pages.len();
        for i in 1..n {
            let mut j = i;
            while j > 0 {
                let swap = match self.pages[j - 1].category.cmp(self.pages[j].category) {
                    core::cmp::Ordering::Greater => true,
                    core::cmp::Ordering::Equal => self.pages[j - 1].order > self.pages[j].order,
                    core::cmp::Ordering::Less => false,
                };
                if swap {
                    self.pages.swap(j - 1, j);
                    j -= 1;
                } else {
                    break;
                }
            }
        }
    }

    /// Return an iterator over the distinct category names, in sorted order.
    ///
    /// Must be called after [`sort`](Self::sort) for correct results.
    pub fn categories(&self) -> impl Iterator<Item = &str> {
        CategoryIter {
            pages: &self.pages,
            pos: 0,
        }
    }
}

/// Iterator that yields each unique category exactly once (assumes sorted input).
struct CategoryIter<'a> {
    pages: &'a [PageInfo],
    pos: usize,
}

impl<'a> Iterator for CategoryIter<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.pages.len() {
            return None;
        }
        let cat = self.pages[self.pos].category;
        // Skip all pages in this category
        while self.pos < self.pages.len() && self.pages[self.pos].category == cat {
            self.pos += 1;
        }
        Some(cat)
    }
}
