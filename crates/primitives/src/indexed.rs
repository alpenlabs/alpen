use borsh::{BorshDeserialize, BorshSerialize};

/// Indexed item. Basically a wrapper around an item with an index.
#[derive(Clone, Debug, BorshDeserialize, BorshSerialize)]
pub struct Indexed<T, Idx = u32> {
    /// Index of the transaction in the block
    index: Idx,

    /// Wrapped item
    item: T,
}

impl<T, Idx> Indexed<T, Idx> {
    /// Creates a new instance.
    pub fn new(index: Idx, item: T) -> Self {
        Self { index, item }
    }

    /// Returns the index of the item
    pub fn index(&self) -> &Idx {
        &self.index
    }

    /// Returns a reference to the item.
    pub fn item(&self) -> &T {
        &self.item
    }

    /// "Unwraps" into the wrapped item.
    pub fn into_item(self) -> T {
        self.item
    }
}
