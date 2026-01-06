use alpen_ee_common::BlockNumHash;
use strata_acct_types::Hash;

pub(crate) fn test_hash(n: u8) -> Hash {
    let mut buf = [0u8; 32];
    buf[0] = 1; // ensure ZERO hash is not created.
    buf[31] = n;
    Hash::from(buf)
}

pub(crate) fn test_blocknumhash(n: u8) -> BlockNumHash {
    BlockNumHash::new(test_hash(n), n as u64)
}
