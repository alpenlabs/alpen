/// Converts satoshis to gwei for EVM compatibility.
///
/// In Alpen: 1 BTC = 10^8 sats = 10^9 gwei
/// Therefore: 1 sat = 10 gwei
///
/// Per EIP-4895, withdrawal amounts are stored in Gwei.
pub fn sats_to_gwei(sats: u64) -> Option<u64> {
    sats.checked_mul(10)
}
