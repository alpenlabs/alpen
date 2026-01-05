use bitcoin::{block::Header, consensus::Encodable};
use strata_crypto::hash::sha256d;
use strata_identifiers::Buf32;

/// Returns the block hash.
///
/// Equivalent to [`compute_block_hash`](Header::block_hash)
/// but internally uses [RustCrypto's SHA-2 crate](https://github.com/RustCrypto/hashes/tree/master/sha2),
/// because it has patches available from both
/// [Risc0](https://github.com/risc0/RustCrypto-hashes)
/// and [Sp1](https://github.com/sp1-patches/RustCrypto-hashes)
pub fn compute_block_hash(header: &Header) -> Buf32 {
    let mut buf = [0u8; 80];
    let mut writer = &mut buf[..];
    header
        .consensus_encode(&mut writer)
        .expect("engines don't error");
    sha256d(&buf)
}

#[cfg(test)]
mod tests {
    use bitcoin::hashes::Hash;
    use strata_test_utils_btc::get_btc_mainnet_block;

    use super::*;

    #[test]
    fn test_compute_block_hash() {
        let btc_block = get_btc_mainnet_block();
        let expected = Buf32::from(btc_block.block_hash().to_raw_hash().to_byte_array());
        let actual = compute_block_hash(&btc_block.header);
        assert_eq!(expected, actual);
    }
}
