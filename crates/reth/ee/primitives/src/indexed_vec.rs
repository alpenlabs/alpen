/// Represents a segment of a list with an incrementing index
#[derive(Debug, Clone)]
pub struct IndexedVec<T> {
    base_idx: u64,
    inner: Vec<T>,
}

impl<T> IndexedVec<T> {
    pub fn from_parts(base_idx: u64, items: Vec<T>) -> Self {
        Self {
            base_idx,
            inner: items,
        }
    }

    pub fn final_idx(&self) -> u64 {
        self.base_idx + self.inner.len() as u64
    }

    pub fn get(&self, idx: u64) -> Option<&T> {
        self.inner.get((idx - self.base_idx) as usize)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (u64, &T)> {
        self.inner
            .iter()
            .enumerate()
            .map(|(i, e)| (self.base_idx + i as u64, e))
    }

    pub fn inner(&self) -> &Vec<T> {
        &self.inner
    }
}
