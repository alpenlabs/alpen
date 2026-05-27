//! Constants used by EE DA proof verification.

/// Magic bytes in the DA commit transaction marker output.
///
/// TODO(STR-1907): derive this from authenticated EE proof context instead of
/// baking the network value into the guest.
pub(super) const EE_DA_MAGIC_BYTES: [u8; 4] = *b"ALPN";

/// Current EE DA blob encoding version.
///
/// TODO(STR-1907): make this part of the same authenticated EE proof context
/// as chain ID and DA magic bytes.
pub(super) const DA_BLOB_VERSION: u32 = 0;
