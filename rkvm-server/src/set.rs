
use core::slice::Iter;
use std::hash::Hash;

pub struct Set<T> {
    data: Vec<T>
}

impl<T> Set<T>
where
    T: Ord {
    pub fn new() -> Self {
        Set { data: Vec::new() }
    }
    pub fn with_capacity(len: usize) -> Self {
        Set { data: Vec::with_capacity(len) }
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn contains(&self, t: &T) -> bool {
        self.data.binary_search(t).is_ok()
    }

    pub fn insert(&mut self, t: T) -> bool {
        match self.data.binary_search(&t) {
            Ok(_) => false,
            Err(i) => {
                self.data.insert(i, t);
                true
            }
        }
    }

    pub fn remove(&mut self, t: &T) -> bool {
        match self.data.binary_search(t) {
            Ok(i) => {
                self.data.remove(i);
                true
            },
            Err(_) => false
        }
    }

    pub fn clear(&mut self) {
        self.data.clear();
    }
}

impl<T: Ord> FromIterator<T> for Set<T> {
    fn from_iter<I:IntoIterator<Item = T>>(iter: I) -> Self {
        let mut data = Vec::from_iter(iter.into_iter());
        data.sort_unstable();
        data.dedup();
        Set { data : data }
    }
}

impl<T: Hash> Hash for Set<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for value in &self.data {
            value.hash(state);
        }
    }
}

impl<T: Eq> PartialEq for Set<T> {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl<T: Eq> Eq for Set<T> {}

impl<T: Clone> Clone for Set<T> {
    fn clone(&self) -> Self {
        Set { data: self.data.clone() }
    }
}
impl<T: Ord> Extend<T> for Set<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.data.extend(iter);
        self.data.sort_unstable();
        self.data.dedup();
    }
}

impl<T> IntoIterator for Set<T> {
    type Item = T;
    type IntoIter = std::vec::IntoIter<T>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Set<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.iter()
    }
}

impl<'a, T> IntoIterator for &'a mut Set<T> {
    type Item = &'a mut T;
    type IntoIter = std::slice::IterMut<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.data.iter_mut()
    }
}