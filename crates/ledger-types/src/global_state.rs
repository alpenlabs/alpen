/// Abstract global chainstate.
pub trait IGlobalState: Clone + PartialEq + Eq {
    /// Gets the current slot.
    fn cur_slot(&mut self) -> u64;

    /// Sets the current slot.
    fn set_cur_slot(&mut self, slot: u64);
}
