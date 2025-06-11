//! parser types for Deposit Tx, and later deposit Request Tx

use bitcoin::{
    hashes::Hash,
    key::TapTweak,
    opcodes::all::OP_RETURN,
    sighash::{Prevouts, SighashCache},
    taproot::TAPROOT_CONTROL_NODE_SIZE,
    Amount, OutPoint, ScriptBuf, TapNodeHash, TapSighashType, Transaction, TxOut, XOnlyPublicKey,
};
use secp256k1::{constants::SCHNORR_SIGNATURE_SIZE, schnorr::Signature, Message};
use strata_primitives::{
    buf::Buf32,
    l1::{DepositInfo, OutputRef},
    prelude::DepositTxParams,
};

use crate::{
    deposit::error::DepositParseError,
    utils::{next_bytes, next_op},
};

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
pub fn extract_deposit_info(tx: &Transaction, config: &DepositTxParams) -> Option<DepositInfo> {
    // Get the first output (index 0)
    let send_addr_out = tx.output.first()?;

    // Get the second output (index 1)
    let op_return_out = tx.output.get(1)?;

    // Check if it is exact deposit denomination amount
    if send_addr_out.value.to_sat() != config.deposit_amount {
        return None;
    }

    // Check if deposit output address matches.
    if send_addr_out.script_pubkey != config.address.address().script_pubkey() {
        return None;
    }

    // Parse the tag from the OP_RETURN output.
    let tag_data = parse_tag_script(&op_return_out.script_pubkey, config).ok()?;

    // Get the first input of the transaction
    let deposit_outpoint = OutputRef::from(OutPoint {
        txid: tx.compute_txid(),
        vout: 0, // deposit must always exist in the first output
    });

    // Check if it was signed off by the operators and hence verify that this is just not someone
    // else sending bitcoin to N-of-N address.
    validate_deposit_signature(tx, &tag_data, config)?;

    // Construct and return the DepositInfo
    Some(DepositInfo {
        deposit_idx: tag_data.deposit_idx,
        amt: send_addr_out.value.into(),
        address: tag_data.dest_buf.to_vec(),
        outpoint: deposit_outpoint,
    })
}

/// Validate that the transaction has been signed off by the N of N operators pubkey.
fn validate_deposit_signature(
    tx: &Transaction,
    tag_data: &DepositTag<'_>,
    dep_config: &DepositTxParams,
) -> Option<()> {
    // Initialize necessary variables and dependencies
    let secp = secp256k1::SECP256K1;

    // FIXME: Use latest version of `bitcoin` once released. The underlying
    // `bitcoinconsensus==0.106` will have support for taproot validation. So here, we just need
    // to create TxOut from operator pubkeys and tapnode hash and call `tx.verify()`.

    // Extract and validate input signature
    let input = tx.input[0].clone();

    // Check if witness is present.
    if input.witness.is_empty() {
        return None;
    }
    let sig_witness = &input.witness[0];
    if sig_witness.len() < SCHNORR_SIGNATURE_SIZE {
        return None;
    }
    let sig_bytes = &sig_witness[..SCHNORR_SIGNATURE_SIZE];
    let schnorr_sig = Signature::from_slice(sig_bytes).ok()?;

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
        .taproot_key_spend_signature_hash(0, &prevout, TapSighashType::All)
        .unwrap();

    // Prepare the message for signature verification
    let msg = Message::from_digest(*sighash.as_byte_array());

    // Verify the Schnorr signature
    secp.verify_schnorr(&schnorr_sig, &msg, &tweaked_key.to_x_only_public_key())
        .ok()
}

struct DepositTag<'buf> {
    deposit_idx: u32,
    dest_buf: &'buf [u8],
    // TODO: better naming
    amount: u64,
    tapscript_root: Buf32,
}

/// extracts the EE address given that the script is OP_RETURN type and contains the Magic Bytes
fn parse_tag_script<'a>(
    script: &'a ScriptBuf,
    config: &DepositTxParams,
) -> Result<DepositTag<'a>, DepositParseError> {
    let mut instructions = script.instructions();

    // Check if OP_RETURN is present and if not just discard it.
    if next_op(&mut instructions) != Some(OP_RETURN) {
        return Err(DepositParseError::MissingTag);
    }

    // Extract the data from the next push.
    let Some(data) = next_bytes(&mut instructions) else {
        return Err(DepositParseError::NoData);
    };

    // If it's not a standard tx then something is *probably* up.
    if data.len() > 80 {
        return Err(DepositParseError::TagOversized);
    }

    parse_tag(data, &config.magic_bytes, config.address_length)
}

/// Parses the script buffer which has the following structure:
/// [magic_bytes(n bytes), stake_idx(4 bytes), ee_address(m bytes), takeback_hash(32 bytes),
/// sats_amt(8 bytes)]
fn parse_tag<'b>(
    buf: &'b [u8],
    magic_bytes: &[u8],
    addr_len: u8,
) -> Result<DepositTag<'b>, DepositParseError> {
    // data has expected magic bytes
    let magic_len = magic_bytes.len();

    if buf.len()
        != magic_len + DEPOSIT_IDX_LEN + SATS_AMOUNT_LEN + TAKEBACK_HASH_LEN + addr_len as usize
    {
        return Err(DepositParseError::InvalidData);
    }

    let (magic_slice, idx_ee_takeback_amt) = buf.split_at(magic_len);
    if magic_slice != magic_bytes {
        return Err(DepositParseError::InvalidMagic);
    }

    // Extract the deposit idx. Can use expect because of the above length check
    let (didx_buf, ee_takeback_amt) = idx_ee_takeback_amt.split_at(DEPOSIT_IDX_LEN);
    let deposit_idx =
        u32::from_be_bytes(didx_buf.try_into().expect("Expect dep idx to be 4 bytes"));

    let (dest_buf, takeback_and_amt) = ee_takeback_amt.split_at(addr_len as usize);

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

    use bitcoin::{
        opcodes::all::OP_RETURN,
        script::{Builder, PushBytesBuf},
        Network,
    };
    use strata_primitives::{
        l1::{BitcoinAddress, XOnlyPk},
        params::DepositTxParams,
    };

    use crate::deposit::{
        deposit_tx::{
            parse_tag, parse_tag_script, DEPOSIT_IDX_LEN, SATS_AMOUNT_LEN, TAKEBACK_HASH_LEN,
        },
        error::DepositParseError,
    };

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
        let magic = [1, 2, 3, 4, 5];
        const ADDR_LEN: usize = 20;

        let deposit_idx: u32 = 42;
        let dest_buf = vec![0xAB; ADDR_LEN];
        let takeback_hash = vec![0xCD; 32];
        let sats_amt: u64 = 1_000_000;

        let mut buf = Vec::new();
        buf.extend_from_slice(&magic);
        buf.extend_from_slice(&deposit_idx.to_be_bytes());
        buf.extend_from_slice(&dest_buf);
        buf.extend_from_slice(&takeback_hash);
        buf.extend_from_slice(&sats_amt.to_be_bytes());

        let result = parse_tag(&buf, &magic, ADDR_LEN as u8).expect("should parse successfully");

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
    fn fails_if_magic_mismatch() {
        let magic = [1, 2, 3, 4, 5];
        const ADDR_LEN: usize = 20;

        let mut bad_buf = Vec::from(b"badmg"); // wrong magic, but correct length
        bad_buf
            .extend_from_slice(&[0u8; DEPOSIT_IDX_LEN + 20 + TAKEBACK_HASH_LEN + SATS_AMOUNT_LEN]);

        let result = parse_tag(&bad_buf, &magic, ADDR_LEN as u8);

        assert!(matches!(result, Err(DepositParseError::InvalidMagic)));
    }

    #[test]
    fn fails_if_buffer_too_short() {
        let magic = [1, 2, 3, 4, 5];
        const ADDR_LEN: usize = 20;
        let short_buf = Vec::from(magic); // only magic, missing everything else

        let result = parse_tag(&short_buf, &magic, ADDR_LEN as u8);

        assert!(matches!(result, Err(DepositParseError::InvalidData)));
    }

    #[test]
    fn fails_if_address_length_mismatch() {
        let magic = [1, 2, 3, 4, 5];
        const ADDR_LEN: usize = 20;

        let deposit_idx: u32 = 10;
        let wrong_dest_buf = vec![0xFF; ADDR_LEN - 1]; // wrong address size
        let takeback_hash = vec![0xCD; 32];
        let sats_amt: u64 = 42;

        let mut buf = Vec::new();
        buf.extend_from_slice(&magic);
        buf.extend_from_slice(&deposit_idx.to_be_bytes());
        buf.extend_from_slice(&wrong_dest_buf);
        buf.extend_from_slice(&takeback_hash);
        buf.extend_from_slice(&sats_amt.to_be_bytes());

        let result = parse_tag(&buf, &magic, ADDR_LEN as u8);

        assert!(matches!(result, Err(DepositParseError::InvalidData)));
    }

    // Tets for parse_tag_script
    #[test]
    fn fails_if_missing_op_return() {
        // Script without OP_RETURN
        let script = Builder::new()
            .push_slice(b"some data") // just pushes data, no OP_RETURN
            .into_script();
        let config = dummy_config();

        let res = parse_tag_script(&script, &config);

        assert!(matches!(res, Err(DepositParseError::MissingTag)));
    }

    #[test]
    fn fails_if_no_data_after_op_return() {
        // Script with OP_RETURN but no pushdata
        let script = Builder::new().push_opcode(OP_RETURN).into_script();
        let config = dummy_config();

        let res = parse_tag_script(&script, &config);

        assert!(matches!(res, Err(DepositParseError::NoData)));
    }

    #[test]
    fn fails_if_tag_data_oversized() {
        // Script with OP_RETURN and oversized pushdata (>80 bytes)
        let oversized_payload = vec![0xAAu8; 81];
        let script = Builder::new()
            .push_opcode(OP_RETURN)
            .push_slice(PushBytesBuf::try_from(oversized_payload).unwrap())
            .into_script();
        let config = dummy_config();

        let res = parse_tag_script(&script, &config);

        assert!(matches!(res, Err(DepositParseError::TagOversized)));
    }

    #[test]
    fn succeeds_if_valid_op_return_and_data() {
        // Script with OP_RETURN and valid size pushdata
        let valid_payload = vec![0xAAu8; 50]; // size < 80 bytes
        let script = Builder::new()
            .push_opcode(OP_RETURN)
            .push_slice(PushBytesBuf::try_from(valid_payload).unwrap())
            .into_script();
        let config = dummy_config();

        // Might still fail inside parse_tag (e.g., InvalidMagic), but must NOT fail for
        // MissingTag/NoData/TagOversized
        let res = parse_tag_script(&script, &config);

        assert!(!matches!(
            res,
            Err(DepositParseError::MissingTag)
                | Err(DepositParseError::NoData)
                | Err(DepositParseError::TagOversized)
        ));
    }
}
