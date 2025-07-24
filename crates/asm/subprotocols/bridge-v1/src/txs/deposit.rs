//! parser types for Deposit Tx, and later deposit Request Tx

use bitcoin::{
    Amount, OutPoint, ScriptBuf, TapNodeHash, Transaction, TxOut, XOnlyPublicKey,
    hashes::Hash,
    key::TapTweak,
    sighash::{Prevouts, SighashCache},
    taproot::{self, TAPROOT_CONTROL_NODE_SIZE},
};
use secp256k1::Message;
use strata_asm_common::TxInputRef;
use strata_l1_txfmt::TagDataRef;
use strata_primitives::{
    buf::Buf32,
    l1::{DepositInfo, OutputRef},
    prelude::DepositTxParams,
};

use crate::errors::DepositParseError;

const TAKEBACK_HASH_LEN: usize = TAPROOT_CONTROL_NODE_SIZE;
const SATS_AMOUNT_LEN: usize = size_of::<u64>();
const DEPOSIT_IDX_LEN: usize = size_of::<u32>();

/// Extracts [`DepositInfo`] from a [`Transaction`].
///
/// This function checks the first output of the transaction to see if it matches the expected
/// deposit amount and address. It also checks the second output for the OP_RETURN script and parses
/// it to extract the deposit index and destination address. Finally, it validates the deposit
/// signature to ensure that the transaction was signed by the operators. If all checks pass, it
/// returns a `DepositInfo` struct containing the deposit index, amount, destination address, and
/// outpoint of the deposit transaction.
///
/// # Errors
///
/// Returns `DepositParseError` if:
/// - The transaction doesn't have the expected output structure
/// - The deposit amount doesn't match the expected amount
/// - The deposit address doesn't match the expected address
/// - The SPS-50 tag parsing fails
/// - The signature validation fails
pub fn extract_deposit_info<'a>(
    tx_input: &TxInputRef<'a>,
    config: &DepositTxParams,
) -> Result<DepositInfo, DepositParseError> {
    // Get the second output (index 1)
    // First output needs to be the OP_RETURN output according to SPS-50.
    let send_addr_out = tx_input
        .tx()
        .output
        .get(1)
        .ok_or(DepositParseError::MissingOutput(1))?;

    // Check if it is exact deposit denomination amount
    if send_addr_out.value.to_sat() != config.deposit_amount {
        return Err(DepositParseError::InvalidDepositAmount {
            expected: config.deposit_amount,
            actual: send_addr_out.value.to_sat(),
        });
    }

    // Check if deposit output address matches.
    if send_addr_out.script_pubkey != config.address.address().script_pubkey() {
        return Err(DepositParseError::InvalidDepositAddress);
    }

    // Parse the tag from the OP_RETURN output.
    let tag_data = parse_tag(tx_input.tag(), config.address_length)?;

    // Get the first input of the transaction
    let deposit_outpoint = OutputRef::from(OutPoint {
        txid: tx_input.tx().compute_txid(),
        vout: 0, // deposit must always exist in the first output
    });

    // Check if it was signed off by the operators and hence verify that this is just not someone
    // else sending bitcoin to N-of-N address.
    validate_deposit_signature(tx_input.tx(), &tag_data, config)?;

    // Construct and return the DepositInfo
    Ok(DepositInfo {
        deposit_idx: tag_data.deposit_idx,
        amt: send_addr_out.value.into(),
        address: tag_data.dest_buf.to_vec(),
        outpoint: deposit_outpoint,
    })
}

/// Validate that the transaction has been signed off by the N-of-N operators pubkey.
fn validate_deposit_signature(
    tx: &Transaction,
    tag_data: &DepositTag<'_>,
    dep_config: &DepositTxParams,
) -> Result<(), DepositParseError> {
    // Initialize necessary variables and dependencies
    let secp = secp256k1::SECP256K1;

    // FIXME: Use latest version of `bitcoin` once released. The underlying
    // `bitcoinconsensus==0.106` will have support for taproot validation. So here, we just need
    // to create TxOut from operator pubkeys and tapnode hash and call `tx.verify()`.

    // Extract and validate input signature
    let input = tx.input[0].clone();

    // Check if witness is present.
    if input.witness.is_empty() {
        return Err(DepositParseError::InvalidSignature);
    }
    let sig_witness = &input.witness[0];

    // rust-bitcoin taproot::Signature handles both both 64-byte (SIGHASH_DEFAULT)
    // and 65-byte (explicit sighash) signatures.
    let taproot_sig = taproot::Signature::from_slice(sig_witness)
        .map_err(|_| DepositParseError::InvalidSignature)?;
    let schnorr_sig = taproot_sig.signature;
    let sighash_type = taproot_sig.sighash_type;

    // Parse the internal pubkey and merkle root
    let internal_pubkey = dep_config.operators_pubkey;
    let merkle_root: TapNodeHash = TapNodeHash::from_byte_array(*tag_data.tapscript_root.as_ref());

    let int_key = XOnlyPublicKey::from_slice(internal_pubkey.inner().as_bytes()).unwrap();
    let (tweaked_key, _) = int_key.tap_tweak(secp, Some(merkle_root));

    // Build the scriptPubKey for the UTXO
    let script_pubkey = ScriptBuf::new_p2tr(secp, int_key, Some(merkle_root));

    let utxos = [TxOut {
        value: Amount::from_sat(tag_data.amount),
        script_pubkey,
    }];

    // Compute the sighash
    let prevout = Prevouts::All(&utxos);
    let sighash = SighashCache::new(tx)
        // NOTE: preserving the original sighash_type.
        .taproot_key_spend_signature_hash(0, &prevout, sighash_type)
        .unwrap();

    // Prepare the message for signature verification
    let msg = Message::from_digest(*sighash.as_byte_array());

    // Verify the Schnorr signature
    secp.verify_schnorr(&schnorr_sig, &msg, &tweaked_key.to_x_only_public_key())
        .map_err(|_| DepositParseError::InvalidSignature)?;

    Ok(())
}

struct DepositTag<'buf> {
    deposit_idx: u32,
    dest_buf: &'buf [u8],
    // TODO: better naming
    amount: u64,
    tapscript_root: Buf32,
}

/// SPS-50 already parses some information from the tag, so we just extract the rest here
fn parse_tag<'b>(
    tag: &'b TagDataRef<'b>,
    addr_len: u8,
) -> Result<DepositTag<'b>, DepositParseError> {
    let aux_data = tag.aux_data();

    if aux_data.len() != DEPOSIT_IDX_LEN + SATS_AMOUNT_LEN + TAKEBACK_HASH_LEN + addr_len as usize {
        return Err(DepositParseError::InvalidData);
    }

    // Extract the deposit idx. Can use expect because of the above length check
    let (didx_buf, rest) = aux_data.split_at(DEPOSIT_IDX_LEN);
    let deposit_idx =
        u32::from_be_bytes(didx_buf.try_into().expect("Expect dep idx to be 4 bytes"));

    let (dest_buf, takeback_and_amt) = rest.split_at(addr_len as usize);

    // Check dest_buf len
    if dest_buf.len() != addr_len as usize {
        return Err(DepositParseError::InvalidDestLen(dest_buf.len() as u8));
    }

    // Extract takeback and amt
    let (takeback_hash, amt) = takeback_and_amt.split_at(TAKEBACK_HASH_LEN);

    // Extract sats, can use expect here because by the initial check on the buf len, we can ensure
    // this.
    let amt_bytes: [u8; 8] = amt
        .try_into()
        .expect("Expected to have 8 bytes as sats amount");

    let sats_amt = u64::from_be_bytes(amt_bytes);

    Ok(DepositTag {
        deposit_idx,
        dest_buf,
        amount: sats_amt,
        tapscript_root: takeback_hash
            .try_into()
            .expect("expected takeback hash length to match"),
    })
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use strata_l1_txfmt::TagDataRef;
    use strata_primitives::{
        l1::{BitcoinAddress, XOnlyPk},
        params::DepositTxParams,
    };

    use super::parse_tag;
    use crate::errors::DepositParseError;

    const MAGIC_BYTES: &[u8] = &[1, 2, 3, 4];
    const ADDRESS: &str = "bcrt1p729l9680ht3zf7uhl6pgdrlhfp9r29cwajr5jk3k05fer62763fscz0w4s";

    fn dummy_config() -> DepositTxParams {
        assert_eq!(MAGIC_BYTES.len(), 4, "test: magic not 4 bytes");
        let addr = BitcoinAddress::parse(ADDRESS, Network::Regtest).unwrap();
        DepositTxParams {
            magic_bytes: MAGIC_BYTES.to_vec(),
            address_length: 20,
            deposit_amount: 10,
            address: addr.clone(),
            operators_pubkey: XOnlyPk::from_address(&addr).unwrap(),
        }
    }

    // Tests for parse_tag

    #[test]
    fn parses_valid_buffer_correctly() {
        const ADDR_LEN: usize = 20;
        const SUBPROTO_ID: u8 = 1;
        const TX_TYPE: u8 = 0; // Deposit tx type

        let deposit_idx: u32 = 42;
        let dest_buf = vec![0xAB; ADDR_LEN];
        let takeback_hash = vec![0xCD; 32];
        let sats_amt: u64 = 1_000_000;

        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&deposit_idx.to_be_bytes());
        aux_data.extend_from_slice(&dest_buf);
        aux_data.extend_from_slice(&takeback_hash);
        aux_data.extend_from_slice(&sats_amt.to_be_bytes());

        let tag_data =
            TagDataRef::new(SUBPROTO_ID, TX_TYPE, &aux_data).expect("Failed to create TagDataRef");

        let result = parse_tag(&tag_data, ADDR_LEN as u8).expect("should parse successfully");

        assert_eq!(result.deposit_idx, 42);
        assert_eq!(result.dest_buf, dest_buf.as_slice());
        assert_eq!(result.amount, sats_amt);
        assert_eq!(
            result.tapscript_root,
            takeback_hash
                .as_slice()
                .try_into()
                .expect("takeback not 32 bytes")
        );
    }

    #[test]
    fn fails_if_buffer_too_short() {
        const ADDR_LEN: usize = 20;
        const SUBPROTO_ID: u8 = 1;
        const TX_TYPE: u8 = 0;
        let short_aux_data = vec![0u8; 10]; // too short

        let tag_data = TagDataRef::new(SUBPROTO_ID, TX_TYPE, &short_aux_data)
            .expect("Failed to create TagDataRef");
        let result = parse_tag(&tag_data, ADDR_LEN as u8);

        assert!(matches!(result, Err(DepositParseError::InvalidData)));
    }

    #[test]
    fn fails_if_address_length_mismatch() {
        const ADDR_LEN: usize = 20;
        const SUBPROTO_ID: u8 = 1;
        const TX_TYPE: u8 = 0;

        let deposit_idx: u32 = 10;
        let wrong_dest_buf = vec![0xFF; ADDR_LEN - 1]; // wrong address size  
        let takeback_hash = vec![0xCD; 32];
        let sats_amt: u64 = 42;

        let mut aux_data = Vec::new();
        aux_data.extend_from_slice(&deposit_idx.to_be_bytes());
        aux_data.extend_from_slice(&wrong_dest_buf);
        aux_data.extend_from_slice(&takeback_hash);
        aux_data.extend_from_slice(&sats_amt.to_be_bytes());

        let tag_data =
            TagDataRef::new(SUBPROTO_ID, TX_TYPE, &aux_data).expect("Failed to create TagDataRef");
        let result = parse_tag(&tag_data, ADDR_LEN as u8);

        assert!(matches!(result, Err(DepositParseError::InvalidData)));
    }
}
