//! Key derivation for Strata bridge operators
//!
//! Bridge operators guarantee the security assumptions of the Strata BitVM-based
//! bridge by enforcing that all peg-ins and peg-outs are valid.
//!
//! Operators are responsible for their own keys and master [`Xpriv`] is not
//! shared between operators. Hence, this crate has a BYOK (Bring Your Own Key) design.
//!
//! They use a set of keys to sign messages and bitcoin transactions.
//! The keys are derived from a master [`Xpriv`]
//! using a [BIP-32](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki)
//! HD derivation path.
//!
//! The derivation paths are:
//!
//! - `m/20000'/20'/100'` for the message signing key
//! - `m/20000'/20'/101'` for the wallet transaction signing key

use bitcoin::bip32::{ChildNumber, Xpriv, Xpub};
use secp256k1::SECP256K1;
use strata_primitives::constants::{
    STRATA_OPERATOR_BASE_DERIVATION_PATH, STRATA_OPERATOR_MESSAGE_IDX, STRATA_OPERATOR_WALLET_IDX,
    STRATA_OP_MESSAGE_DERIVATION_PATH, STRATA_OP_WALLET_DERIVATION_PATH,
};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::error::KeyError;

/// Operator's message signing and wallet transaction signing _private_ keys.
#[derive(Debug, Clone)]
pub struct OperatorKeys {
    /// Operator's master [`Xpriv`].
    master: Xpriv,

    /// Operator's base [`Xpriv`].
    ///
    /// # Notes
    ///
    /// This is the [`Xpriv`] that is generated from only hardened paths.
    base: Xpriv,

    /// Operator's message signing [`Xpriv`].
    message: Xpriv,

    /// Operator's wallet transaction signing [`Xpriv`].
    wallet: Xpriv,
}

impl OperatorKeys {
    /// Creates a new [`OperatorKeys`] from a master [`Xpriv`].
    pub fn new(master: &Xpriv) -> Result<Self, KeyError> {
        let base_xpriv = master.derive_priv(SECP256K1, &*STRATA_OPERATOR_BASE_DERIVATION_PATH)?;
        let message_xpriv = master.derive_priv(SECP256K1, &*STRATA_OP_MESSAGE_DERIVATION_PATH)?;
        let wallet_xpriv = master.derive_priv(SECP256K1, &*STRATA_OP_WALLET_DERIVATION_PATH)?;

        Ok(Self {
            master: *master,
            base: base_xpriv,
            message: message_xpriv,
            wallet: wallet_xpriv,
        })
    }

    /// Operator's master [`Xpriv`].
    pub const fn master_xpriv(&self) -> &Xpriv {
        &self.master
    }

    /// Operator's base [`Xpriv`].
    ///
    /// # Notes
    ///
    /// This is the [`Xpriv`] that is generated from only hardened paths from the master [`Xpriv`].
    pub const fn base_xpriv(&self) -> &Xpriv {
        &self.base
    }

    /// Operator's wallet transaction signing [`Xpriv`].
    pub const fn wallet_xpriv(&self) -> &Xpriv {
        &self.wallet
    }

    /// Operator's message signing [`Xpriv`].
    pub const fn message_xpriv(&self) -> &Xpriv {
        &self.message
    }

    /// Operator's master [`Xpub`].
    pub fn master_xpub(&self) -> Xpub {
        Xpub::from_priv(SECP256K1, &self.master)
    }

    /// Operator's base [`Xpub`]
    ///
    /// Infallible conversion from [`Xpriv`] to [`Xpub`] according to
    /// [BIP-32](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki)
    ///
    /// # Notes
    ///
    /// This is the [`Xpub`] that is generated from only hardened paths from the master [`Xpriv`].
    pub fn base_xpub(&self) -> Xpub {
        Xpub::from_priv(SECP256K1, &self.base)
    }

    /// Operator's message signing [`Xpub`].
    ///
    /// Infallible according to
    /// [BIP-32](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki)
    pub fn message_xpub(&self) -> Xpub {
        Xpub::from_priv(SECP256K1, &self.message)
    }

    /// Operator's wallet transaction signing [`Xpub`].
    ///
    /// Infallible conversion from [`Xpriv`] to [`Xpub`] according to
    /// [BIP-32](https://github.com/bitcoin/bips/blob/master/bip-0032.mediawiki)
    pub fn wallet_xpub(&self) -> Xpub {
        Xpub::from_priv(SECP256K1, &self.wallet)
    }
}

// Manual Drop implementation to zeroize keys on drop.
impl Drop for OperatorKeys {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl Zeroize for OperatorKeys {
    #[inline]
    fn zeroize(&mut self) {
        let Self {
            master,
            base,
            message,
            wallet,
        } = self;

        // # Security note
        //
        // Going over all possible "zeroizable" fields.
        // What we cannot zeroize is only:
        //
        // - Network: enum
        //
        // These are fine to leave as they are since they are public parameters,
        // and not secret values.
        //
        // NOTE: `Xpriv.private_key` (`SecretKey`) `non_secure_erase` writes `1`s to the memory.

        // Zeroize master components
        master.depth.zeroize();
        {
            let fingerprint: &mut [u8; 4] = master.parent_fingerprint.as_mut();
            fingerprint.zeroize();
        }
        master.private_key.non_secure_erase();
        {
            let chaincode: &mut [u8; 32] = master.chain_code.as_mut();
            chaincode.zeroize();
        }
        let raw_ptr = &mut master.child_number as *mut ChildNumber;
        // SAFETY: `master.child_number` is a valid enum variant
        //          and will not be accessed after zeroization.
        //          Also there are only two possible variants that will
        //          always have an `index` which is a `u32`.
        //          Note that `ChildNumber` does not have the `#[non_exhaustive]`
        //          attribute.
        unsafe {
            *raw_ptr = if master.child_number.is_normal() {
                ChildNumber::Normal { index: 0 }
            } else {
                ChildNumber::Hardened { index: 0 }
            };
        }

        // Zeroize base components
        base.depth.zeroize();
        {
            let fingerprint: &mut [u8; 4] = base.parent_fingerprint.as_mut();
            fingerprint.zeroize();
        }
        base.private_key.non_secure_erase();
        {
            let chaincode: &mut [u8; 32] = base.chain_code.as_mut();
            chaincode.zeroize();
        }
        let raw_ptr = &mut base.child_number as *mut ChildNumber;
        // SAFETY: `base.child_number` is a valid enum variant
        //          and will not be accessed after zeroization.
        //          Also there are only two possible variants that will
        //          always have an `index` which is a `u32`.
        //          Note that `ChildNumber` does not have the `#[non_exhaustive]`
        //          attribute.
        unsafe {
            *raw_ptr = if base.child_number.is_normal() {
                ChildNumber::Normal { index: 0 }
            } else {
                ChildNumber::Hardened { index: 0 }
            };
        }

        // Zeroize message components
        message.depth.zeroize();
        {
            let fingerprint: &mut [u8; 4] = message.parent_fingerprint.as_mut();
            fingerprint.zeroize();
        }
        message.private_key.non_secure_erase();
        {
            let chaincode: &mut [u8; 32] = message.chain_code.as_mut();
            chaincode.zeroize();
        }
        let raw_ptr = &mut message.child_number as *mut ChildNumber;
        // SAFETY: `message.child_number` is a valid enum variant
        //          and will not be accessed after zeroization.
        //          Also there are only two possible variants that will
        //          always have an `index` which is a `u32`.
        //          Note that `ChildNumber` does not have the `#[non_exhaustive]`
        //          attribute.
        unsafe {
            *raw_ptr = if message.child_number.is_normal() {
                ChildNumber::Normal { index: 0 }
            } else {
                ChildNumber::Hardened { index: 0 }
            };
        }

        // Zeroize wallet components
        wallet.depth.zeroize();
        {
            let fingerprint: &mut [u8; 4] = wallet.parent_fingerprint.as_mut();
            fingerprint.zeroize();
        }
        wallet.private_key.non_secure_erase();
        {
            let chaincode: &mut [u8; 32] = wallet.chain_code.as_mut();
            chaincode.zeroize();
        }
        let raw_ptr = &mut wallet.child_number as *mut ChildNumber;
        // SAFETY: `wallet.child_number` is a valid enum variant
        //          and will not be accessed after zeroization.
        //          Also there are only two possible variants that will
        //          always have an `index` which is a `u32`.
        //          Note that `ChildNumber` does not have the `#[non_exhaustive]`
        //          attribute.
        unsafe {
            *raw_ptr = if wallet.child_number.is_normal() {
                ChildNumber::Normal { index: 0 }
            } else {
                ChildNumber::Hardened { index: 0 }
            };
        }
    }
}

impl ZeroizeOnDrop for OperatorKeys {}

/// Converts the base [`Xpub`] to the message [`Xpub`].
pub fn convert_base_xpub_to_message_xpub(base_xpub: &Xpub) -> Xpub {
    let message_partial_path = ChildNumber::from_normal_idx(STRATA_OPERATOR_MESSAGE_IDX)
        .expect("unfallible as long MESSAGE_IDX is not changed");
    base_xpub
        .derive_pub(SECP256K1, &message_partial_path)
        .expect("unfallible")
}

/// Converts the base [`Xpub`] to the wallet [`Xpub`].
pub fn convert_base_xpub_to_wallet_xpub(base_xpub: &Xpub) -> Xpub {
    let wallet_partial_path = ChildNumber::from_normal_idx(STRATA_OPERATOR_WALLET_IDX)
        .expect("unfallible as long WALLET_IDX is not changed");
    base_xpub
        .derive_pub(SECP256K1, &wallet_partial_path)
        .expect("unfallible")
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::LazyLock};

    use bitcoin::{
        absolute, bip32::DerivationPath, consensus, hashes::Hash, psbt::Input,
        transaction::Version, Address, Amount, OutPoint, Psbt, Sequence, TapSighashType,
        Transaction, TxIn, TxOut, Txid, Witness,
    };

    use super::*;

    // What's better than bacon? bacon^24 of course
    // Thix xpriv was generated by the bacon^24 mnemonic
    // Don't use this in production!
    const XPRIV_STR: &str = "tprv8ZgxMBicQKsPeh9dSitM82FU7Fz3ZgPkKmmovAr2aqwauAMVgjcEkZBb2etBtRPZ8XYVm7shxcKwVaDus7T5kauJXVsqAfzM4Tty13rRjAG";
    static XPRIV: LazyLock<Xpriv> = LazyLock::new(|| XPRIV_STR.parse().unwrap());

    // The first address derived from the xpriv above using a `tr()` descriptor.
    const ADDRESS: &str = "bcrt1p729l9680ht3zf7uhl6pgdrlhfp9r29cwajr5jk3k05fer62763fscz0w4s";
    // The second address derived from the xpriv above using a `tr()` descriptor.
    const DEST_ADDRESS: &str = "bcrt1p5uhmu40t5yl97kr95s2m4sr8a9f3af2meqeefkx33symwex3wfqqfe77m3";

    // Dummy values for the test.
    const DUMMY_UTXO_AMOUNT: Amount = Amount::from_sat(20_000_000);
    const SPEND_AMOUNT: Amount = Amount::from_sat(19_999_000); // 1000 sat fee.

    #[test]
    fn test_operator_keys() {
        // Parse stuff
        let address = ADDRESS.parse::<Address<_>>().unwrap().assume_checked();
        let dest_address = DEST_ADDRESS.parse::<Address<_>>().unwrap().assume_checked();

        // Create the operator keys
        let operator_keys = OperatorKeys::new(&XPRIV).unwrap();
        let wallet_key = operator_keys.wallet_xpriv();
        let wallet_pubkey = operator_keys.wallet_xpub();
        let wallet_fingerprint = wallet_pubkey.fingerprint();
        let derivation_path = DerivationPath::master();
        let (x_only_pubkey, _) = wallet_pubkey.public_key.x_only_public_key();

        // Create a dummy transaction with a single input and output.
        let outpoint = OutPoint {
            txid: Txid::all_zeros(),
            vout: 0,
        };
        let txin = TxIn {
            previous_output: outpoint,
            sequence: Sequence::ENABLE_RBF_NO_LOCKTIME,
            ..Default::default()
        };
        let txout = TxOut {
            value: SPEND_AMOUNT,
            script_pubkey: dest_address.script_pubkey(),
        };
        let previous_txout = TxOut {
            value: DUMMY_UTXO_AMOUNT,
            script_pubkey: address.script_pubkey(),
        };

        // Create the unsigned transaction
        let transaction = Transaction {
            version: Version::TWO,
            lock_time: absolute::LockTime::ZERO,
            input: vec![txin],
            output: vec![txout],
        };

        // Create the PSBT
        let mut psbt = Psbt::from_unsigned_tx(transaction).expect("could not create PSBT");
        let ty = TapSighashType::All.into();
        let origins = BTreeMap::from([(
            x_only_pubkey,
            (vec![], (wallet_fingerprint, derivation_path)),
        )]);

        // Add the input to the PSBT
        psbt.inputs = vec![Input {
            witness_utxo: Some(previous_txout),
            tap_key_origins: origins,
            tap_internal_key: Some(x_only_pubkey),
            sighash_type: Some(ty),
            ..Default::default()
        }];

        // Sign the PSBT
        psbt.sign(wallet_key, SECP256K1)
            .expect("could not sign PSBT");

        // Finalize the PSBT
        psbt.inputs[0].final_script_witness = Some(Witness::p2tr_key_spend(
            &psbt.inputs[0].tap_key_sig.unwrap(),
        ));
        // Clear all the data fields as per the spec.
        psbt.inputs[0].partial_sigs = BTreeMap::new();
        psbt.inputs[0].sighash_type = None;
        psbt.inputs[0].redeem_script = None;
        psbt.inputs[0].witness_script = None;
        psbt.inputs[0].bip32_derivation = BTreeMap::new();

        // Extract the transaction and serialize it
        let signed_tx = psbt.extract_tx().expect("valid transaction");
        let serialized_signed_tx = consensus::encode::serialize_hex(&signed_tx);
        println!("serialized_signed_tx: {serialized_signed_tx}");
    }

    #[test]
    fn test_zeroize() {
        use bitcoin::Network;

        let master = Xpriv::new_master(Network::Regtest, &[2u8; 32]).unwrap();
        let mut keys = OperatorKeys::new(&master).unwrap();

        // Store original values
        let master_chaincode = *keys.master_xpriv().chain_code.as_bytes();
        let base_chaincode = *keys.base_xpriv().chain_code.as_bytes();
        let message_chaincode = *keys.message_xpriv().chain_code.as_bytes();
        let wallet_chaincode = *keys.wallet_xpriv().chain_code.as_bytes();

        // Verify data exists
        assert_ne!(master_chaincode, [0u8; 32]);
        assert_ne!(base_chaincode, [0u8; 32]);
        assert_ne!(message_chaincode, [0u8; 32]);
        assert_ne!(wallet_chaincode, [0u8; 32]);

        // Manually zeroize
        keys.zeroize();

        // Verify fields are zeroed
        // NOTE: SecretKey::non_secure_erase writes `1`s to the memory.
        assert_eq!(keys.master_xpriv().private_key.secret_bytes(), [1u8; 32]);
        assert_eq!(keys.base_xpriv().private_key.secret_bytes(), [1u8; 32]);
        assert_eq!(keys.message_xpriv().private_key.secret_bytes(), [1u8; 32]);
        assert_eq!(keys.wallet_xpriv().private_key.secret_bytes(), [1u8; 32]);

        assert_eq!(*keys.master_xpriv().chain_code.as_bytes(), [0u8; 32]);
        assert_eq!(*keys.base_xpriv().chain_code.as_bytes(), [0u8; 32]);
        assert_eq!(*keys.message_xpriv().chain_code.as_bytes(), [0u8; 32]);
        assert_eq!(*keys.wallet_xpriv().chain_code.as_bytes(), [0u8; 32]);

        assert_eq!(*keys.master_xpriv().parent_fingerprint.as_bytes(), [0u8; 4]);
        assert_eq!(*keys.base_xpriv().parent_fingerprint.as_bytes(), [0u8; 4]);
        assert_eq!(
            *keys.message_xpriv().parent_fingerprint.as_bytes(),
            [0u8; 4]
        );
        assert_eq!(*keys.wallet_xpriv().parent_fingerprint.as_bytes(), [0u8; 4]);

        assert_eq!(keys.master_xpriv().depth, 0);
        assert_eq!(keys.base_xpriv().depth, 0);
        assert_eq!(keys.message_xpriv().depth, 0);
        assert_eq!(keys.wallet_xpriv().depth, 0);

        // Check if child numbers are zeroed while maintaining their hardened/normal status
        match keys.master_xpriv().child_number {
            ChildNumber::Normal { index } => assert_eq!(index, 0),
            ChildNumber::Hardened { index } => assert_eq!(index, 0),
        }
        match keys.base_xpriv().child_number {
            ChildNumber::Normal { index } => assert_eq!(index, 0),
            ChildNumber::Hardened { index } => assert_eq!(index, 0),
        }
        match keys.message_xpriv().child_number {
            ChildNumber::Normal { index } => assert_eq!(index, 0),
            ChildNumber::Hardened { index } => assert_eq!(index, 0),
        }
        match keys.wallet_xpriv().child_number {
            ChildNumber::Normal { index } => assert_eq!(index, 0),
            ChildNumber::Hardened { index } => assert_eq!(index, 0),
        }
    }
}
