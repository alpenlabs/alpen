use strata_acct_types::{AccountId, AccountSerial};

/// Describes a map of contiguous serials to account IDs, ensuring O(1) lookups.
///
/// This is useful when tracking newly-created accounts.
#[derive(Clone, Debug)]
pub struct SerialMap {
    /// Tracks the serial of the first account ID.  This is invalid if the vec
    /// is empty.
    first: AccountSerial,

    /// Tracks account IDs, ordered by serial.  If this is empty, then the first
    /// serial is unset.
    ids: Vec<AccountId>,
}

impl SerialMap {
    pub fn new() -> Self {
        Self {
            first: AccountSerial::zero(),
            ids: Vec::new(),
        }
    }

    /// Convenience function to create a serial map starting with an existing
    /// account ID.
    pub fn new_first(serial: AccountSerial, id: AccountId) -> Self {
        Self {
            first: serial,
            ids: vec![id],
        }
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// Returns the account IDs in serial order.
    pub fn ids(&self) -> &[AccountId] {
        &self.ids
    }

    fn offset_from_first(&self, off: usize) -> AccountSerial {
        assert!(!self.ids.is_empty(), "serialmap: use first idx");
        offset_serial_by(self.first, off)
    }

    /// Gets the ID of the first-added account, if present.
    pub fn first_id(&self) -> Option<&AccountId> {
        self.ids.first()
    }

    /// Gets the serial of the first-added account, if present.
    pub fn first_serial(&self) -> Option<AccountSerial> {
        if self.ids.is_empty() {
            None
        } else {
            Some(self.first)
        }
    }

    /// Gets the ID of the last-added account, if present.
    pub fn last_id(&self) -> Option<&AccountId> {
        self.ids.last()
    }

    /// Gets the serial of the last-added account, if present.
    pub fn last_serial(&self) -> Option<AccountSerial> {
        if self.ids.is_empty() {
            None
        } else {
            Some(self.offset_from_first(self.ids.len() - 1))
        }
    }

    /// Gets the next expected serial, if known.
    pub fn next_expected_serial(&self) -> Option<AccountSerial> {
        if self.ids.is_empty() {
            None
        } else {
            Some(self.offset_from_first(self.ids.len()))
        }
    }

    /// Checks if a provided serial is valid to be passed to the next invocation
    /// of [`Self::insert_next`].  If there has not been any serials written,
    /// then this check effectively does nothing.
    pub fn check_next_serial(&self, serial: AccountSerial) -> bool {
        self.next_expected_serial()
            .is_none_or(|next| serial == next)
    }

    /// Inserts the next account ID, checking the passed serial.
    ///
    /// Returns if the entry was successfully added or not (ie. if the serial matched).
    pub fn insert_next(&mut self, serial: AccountSerial, id: AccountId) -> bool {
        if !self.check_next_serial(serial) {
            return false;
        }

        // If we are inserting the first entry, then we're also setting the
        // first serial, so we should add that.
        if self.ids.is_empty() {
            self.first = serial;
        }

        self.ids.push(id);
        true
    }

    /// Inserts the next account ID, without checking a serial.
    ///
    /// # Panics
    ///
    /// If there have not been any serials added yet.
    pub fn insert_next_unchecked(&mut self, id: AccountId) {
        assert!(!self.ids.is_empty(), "serialmap: no first account");
        self.ids.push(id);
    }

    /// Performs a linear scan to find the serial of an account ID, if present.
    pub fn find_account_serial(&self, query: &AccountId) -> Option<AccountSerial> {
        let (idx, _) = self.ids.iter().enumerate().find(|(_, id)| *id == query)?;
        Some(self.offset_from_first(idx))
    }

    /// Looks up an account ID by serial in O(1) time.
    pub fn get(&self, serial: AccountSerial) -> Option<&AccountId> {
        if self.ids.is_empty() {
            return None;
        }
        let serial_val = serial.inner();
        let first_val = self.first.inner();
        if serial_val < first_val {
            return None;
        }
        let idx = (serial_val - first_val) as usize;
        self.ids.get(idx)
    }

    /// Returns an iterator over (serial, account_id) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (AccountSerial, &AccountId)> {
        self.ids
            .iter()
            .enumerate()
            .map(|(idx, id)| (self.offset_from_first(idx), id))
    }
}

impl Default for SerialMap {
    fn default() -> Self {
        Self::new()
    }
}

fn offset_serial_by(serial: AccountSerial, amt: usize) -> AccountSerial {
    AccountSerial::from(serial.inner() + amt as u32)
}

#[cfg(test)]
mod tests {
    use strata_acct_types::AccountId;

    use super::*;

    fn test_account_id(seed: u8) -> AccountId {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        AccountId::from(bytes)
    }

    #[test]
    fn test_new_creates_empty_map() {
        let map = SerialMap::new();
        assert!(map.is_empty());
        assert_eq!(map.len(), 0);
        assert_eq!(map.first_id(), None);
        assert_eq!(map.first_serial(), None);
        assert_eq!(map.last_id(), None);
        assert_eq!(map.last_serial(), None);
        assert_eq!(map.next_expected_serial(), None);
    }

    #[test]
    fn test_new_first_creates_map_with_one_entry() {
        let serial = AccountSerial::from(10u32);
        let id = test_account_id(1);
        let map = SerialMap::new_first(serial, id);

        assert!(!map.is_empty());
        assert_eq!(map.len(), 1);
        assert_eq!(map.first_id(), Some(&id));
        assert_eq!(map.first_serial(), Some(serial));
        assert_eq!(map.last_id(), Some(&id));
        assert_eq!(map.last_serial(), Some(serial));
        assert_eq!(map.next_expected_serial(), Some(AccountSerial::from(11u32)));
    }

    #[test]
    fn test_check_next_serial_empty_map() {
        let map = SerialMap::new();
        // Empty map accepts any serial
        assert!(map.check_next_serial(AccountSerial::from(0u32)));
        assert!(map.check_next_serial(AccountSerial::from(100u32)));
    }

    #[test]
    fn test_check_next_serial_non_empty_map() {
        let serial = AccountSerial::from(10u32);
        let id = test_account_id(1);
        let map = SerialMap::new_first(serial, id);

        // Should only accept the next serial (11)
        assert!(!map.check_next_serial(AccountSerial::from(10u32)));
        assert!(map.check_next_serial(AccountSerial::from(11u32)));
        assert!(!map.check_next_serial(AccountSerial::from(12u32)));
    }

    #[test]
    fn test_insert_next_first_entry() {
        let mut map = SerialMap::new();
        let serial = AccountSerial::from(5u32);
        let id = test_account_id(1);

        assert!(map.insert_next(serial, id));
        assert_eq!(map.len(), 1);
        assert_eq!(map.first_serial(), Some(serial));
        assert_eq!(map.first_id(), Some(&id));
    }

    #[test]
    fn test_insert_next_multiple_entries() {
        let mut map = SerialMap::new();
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        // Insert first entry
        assert!(map.insert_next(AccountSerial::from(10u32), id1));
        assert_eq!(map.len(), 1);

        // Insert second entry with correct serial
        assert!(map.insert_next(AccountSerial::from(11u32), id2));
        assert_eq!(map.len(), 2);

        // Insert third entry with correct serial
        assert!(map.insert_next(AccountSerial::from(12u32), id3));
        assert_eq!(map.len(), 3);

        // Verify all serials and IDs
        assert_eq!(map.first_serial(), Some(AccountSerial::from(10u32)));
        assert_eq!(map.last_serial(), Some(AccountSerial::from(12u32)));
        assert_eq!(map.first_id(), Some(&id1));
        assert_eq!(map.last_id(), Some(&id3));
        assert_eq!(map.next_expected_serial(), Some(AccountSerial::from(13u32)));
    }

    #[test]
    fn test_insert_next_wrong_serial_rejected() {
        let mut map = SerialMap::new();
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);

        // Insert first entry
        assert!(map.insert_next(AccountSerial::from(10u32), id1));

        // Try to insert with wrong serial (not 11)
        assert!(!map.insert_next(AccountSerial::from(12u32), id2));
        assert_eq!(map.len(), 1); // Should not have been added

        // Insert with correct serial should work
        assert!(map.insert_next(AccountSerial::from(11u32), id2));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn test_insert_next_unchecked() {
        let mut map = SerialMap::new_first(AccountSerial::from(5u32), test_account_id(1));
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        map.insert_next_unchecked(id2);
        assert_eq!(map.len(), 2);

        map.insert_next_unchecked(id3);
        assert_eq!(map.len(), 3);

        assert_eq!(map.last_id(), Some(&id3));
        assert_eq!(map.last_serial(), Some(AccountSerial::from(7u32)));
    }

    #[test]
    #[should_panic(expected = "serialmap: no first account")]
    fn test_insert_next_unchecked_panics_on_empty() {
        let mut map = SerialMap::new();
        map.insert_next_unchecked(test_account_id(1));
    }

    #[test]
    fn test_find_account_serial_empty_map() {
        let map = SerialMap::new();
        let id = test_account_id(1);
        assert_eq!(map.find_account_serial(&id), None);
    }

    #[test]
    fn test_find_account_serial_single_entry() {
        let serial = AccountSerial::from(10u32);
        let id = test_account_id(1);
        let map = SerialMap::new_first(serial, id);

        assert_eq!(map.find_account_serial(&id), Some(serial));
        assert_eq!(map.find_account_serial(&test_account_id(2)), None);
    }

    #[test]
    fn test_find_account_serial_multiple_entries() {
        let mut map = SerialMap::new();
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        map.insert_next(AccountSerial::from(100u32), id1);
        map.insert_next(AccountSerial::from(101u32), id2);
        map.insert_next(AccountSerial::from(102u32), id3);

        // Find each account
        assert_eq!(
            map.find_account_serial(&id1),
            Some(AccountSerial::from(100u32))
        );
        assert_eq!(
            map.find_account_serial(&id2),
            Some(AccountSerial::from(101u32))
        );
        assert_eq!(
            map.find_account_serial(&id3),
            Some(AccountSerial::from(102u32))
        );

        // Non-existent account
        assert_eq!(map.find_account_serial(&test_account_id(4)), None);
    }

    #[test]
    fn test_first_and_last_with_multiple_entries() {
        let mut map = SerialMap::new();
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        map.insert_next(AccountSerial::from(50u32), id1);
        map.insert_next(AccountSerial::from(51u32), id2);
        map.insert_next(AccountSerial::from(52u32), id3);

        assert_eq!(map.first_id(), Some(&id1));
        assert_eq!(map.first_serial(), Some(AccountSerial::from(50u32)));
        assert_eq!(map.last_id(), Some(&id3));
        assert_eq!(map.last_serial(), Some(AccountSerial::from(52u32)));
    }

    #[test]
    fn test_next_expected_serial_progression() {
        let mut map = SerialMap::new();

        // Empty map has no expected serial
        assert_eq!(map.next_expected_serial(), None);

        // After first insert
        map.insert_next(AccountSerial::from(10u32), test_account_id(1));
        assert_eq!(map.next_expected_serial(), Some(AccountSerial::from(11u32)));

        // After second insert
        map.insert_next(AccountSerial::from(11u32), test_account_id(2));
        assert_eq!(map.next_expected_serial(), Some(AccountSerial::from(12u32)));

        // After third insert
        map.insert_next(AccountSerial::from(12u32), test_account_id(3));
        assert_eq!(map.next_expected_serial(), Some(AccountSerial::from(13u32)));
    }

    #[test]
    fn test_iter() {
        let mut map = SerialMap::new();
        let id1 = test_account_id(1);
        let id2 = test_account_id(2);
        let id3 = test_account_id(3);

        // Empty map yields nothing
        assert_eq!(map.iter().count(), 0);

        map.insert_next(AccountSerial::from(100u32), id1);
        map.insert_next(AccountSerial::from(101u32), id2);
        map.insert_next(AccountSerial::from(102u32), id3);

        let entries: Vec<_> = map.iter().collect();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], (AccountSerial::from(100u32), &id1));
        assert_eq!(entries[1], (AccountSerial::from(101u32), &id2));
        assert_eq!(entries[2], (AccountSerial::from(102u32), &id3));
    }
}
