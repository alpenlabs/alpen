/// Abstract global chainstate.
pub trait IGlobalState: Clone {
    /// Gets the current epoch.
    fn cur_epoch(&self) -> u64;

    /// Sets the current epoch.
    fn set_cur_epoch(&mut self, epoch: u64);

    /// Gets the current slot.
    fn cur_slot(&self) -> u64;

    /// Sets the current slot.
    fn set_cur_slot(&mut self, slot: u64);
}
