use std::sync::LazyLock;

use bdk_wallet::bitcoin::{Amount, Network};

/// Magic bytes to add to the metadata output in transactions to help identify them.
pub(crate) const MAGIC_BYTES: &[u8; 4] = b"ALPN";

/// 10 BTC + 0.01 to cover fees in the following transaction where the operator spends it into the
/// federation.
pub(crate) const BRIDGE_IN_AMOUNT: Amount = Amount::from_sat(1_001_000_000);

/// Bridge outs are enforced to be exactly 10 BTC
#[allow(dead_code)] // TODO: Remove this when bridge out is implemented
pub(crate) const BRIDGE_OUT_AMOUNT: Amount = Amount::from_int_btc(10);

/// An xpriv that is good enough for testing purposes.
///
/// # Warning
///
/// This is a hardcoded xpriv key that should not be used in production.
// taken from https://github.com/rust-bitcoin/rust-bitcoin/blob/bb38aeb786f408247d5bbc88b9fa13616c74c009/bitcoin/examples/taproot-psbt.rs#L18C38-L18C149
pub(crate) const XPRIV: &str = "tprv8ZgxMBicQKsPd4arFr7sKjSnKFDVMR2JHw9Y8L9nXN4kiok4u28LpHijEudH3mMYoL4pM5UL9Bgdz2M4Cy8EzfErmU9m86ZTw6hCzvFeTg7";

/// The network to use for the tests.
pub(crate) const NETWORK: Network = Network::Regtest;

/// The Taproot-enable wallet's external descriptor.
pub(crate) static DESCRIPTOR: LazyLock<&'static str> =
    LazyLock::new(|| Box::leak(format!("tr({XPRIV}/86'/1'/0'/0/*)").into_boxed_str()));

/// The Taproot-enable wallet's internal descriptor.
pub(crate) static CHANGE_DESCRIPTOR: LazyLock<&'static str> =
    LazyLock::new(|| Box::leak(format!("tr({XPRIV}/86'/1'/0'/1/*)").into_boxed_str()));

#[cfg(test)]
mod tests {
    use bdk_wallet::bitcoin::Address;
    use secp256k1::SECP256K1;
    use strata_primitives::constants::UNSPENDABLE_PUBLIC_KEY;

    use super::*;

    #[test]
    fn unspendable() {
        let unspendable_address = Address::p2tr(SECP256K1, *UNSPENDABLE_PUBLIC_KEY, None, NETWORK);
        assert_eq!(
            unspendable_address.to_string(),
            "bcrt1p7hgsjwtz2pkz45y97dglj4yuc88zsva2p0n5tmcz0zrvfmhcc2lsckedfk"
        );
    }
}
