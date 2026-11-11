// Port of dng_string_list.h/.cpp — a simple list of dng_string.

use crate::dng_string::DngString;
use crate::dng_types::uint32;

/// Port of `dng_string_list`.
#[derive(Debug, Clone, Default)]
pub struct DngStringList {
    list: Vec<DngString>,
}

impl DngStringList {
    pub fn new() -> Self {
        DngStringList::default()
    }

    /// `Count ()`.
    pub fn count(&self) -> uint32 {
        self.list.len() as uint32
    }

    /// `operator[]`.
    pub fn get(&self, index: uint32) -> &DngString {
        &self.list[index as usize]
    }

    /// `operator[]` mutable.
    pub fn get_mut(&mut self, index: uint32) -> &mut DngString {
        &mut self.list[index as usize]
    }

    /// `Allocate (minSize)` — reserve only.
    pub fn allocate(&mut self, min_size: uint32) {
        self.list.reserve(min_size as usize);
    }

    /// `Insert (index, s)`.
    pub fn insert(&mut self, index: uint32, s: &DngString) {
        let index = (index as usize).min(self.list.len());
        self.list.insert(index, s.clone());
    }

    /// `Append (s)`.
    pub fn append(&mut self, s: &DngString) {
        self.insert(self.count(), s);
    }

    /// `Contains (s)`.
    pub fn contains(&self, s: &DngString) -> bool {
        self.list.iter().any(|x| x == s)
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        self.list.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_basics() {
        let mut l = DngStringList::new();
        l.append(&DngString::from_bytes(b"a"));
        l.append(&DngString::from_bytes(b"c"));
        l.insert(1, &DngString::from_bytes(b"b"));
        assert_eq!(l.count(), 3);
        assert_eq!(l.get(1).get(), b"b");
        assert!(l.contains(&DngString::from_bytes(b"c")));
        l.clear();
        assert_eq!(l.count(), 0);
    }
}
