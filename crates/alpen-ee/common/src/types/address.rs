//! EVM-style address and identifiers.

/// Type alias to distinguish EVM addresses.
pub(crate) type EvmAddress = [u8; 20];

/// Type alias to distinguish EVM account storage slots.
pub(crate) type EvmSlot = [u8; 32];
