//! parser types for Deposit Tx, and later deposit Request Tx

use std::convert::TryInto;

use bdk_wallet::bitcoin::{
    opcodes::{all::OP_RETURN, Opcode},
    script::{Instruction, Instructions},
    ScriptBuf,
    Transaction
};
use strata_primitives::l1::DepositRequestInfo;
use strata_params::DepositTxParams;
use tracing::debug;

struct DepositRequestScriptInfo {
    pub tap_ctrl_blk_hash: [u8; 32],
    pub ee_bytes: Vec<u8>,
}

/// Extracts the DepositInfo from the Deposit Transaction
pub(crate) fn extract_deposit_request_info(
    tx: &Transaction,
    config: &DepositTxParams,
) -> Option<DepositRequestInfo> {
    // Ensure that the transaction has at least 2 outputs
    let addr_txn = tx.output.first()?;
    let op_return_txn = tx.output.get(1)?;

    // Parse the deposit request script from the second output's script_pubkey
    let DepositRequestScriptInfo {
        tap_ctrl_blk_hash,
        ee_bytes,
    } = parse_deposit_request_script(&op_return_txn.script_pubkey, config)?;

    // if sent value is less than equal to what we expect for bridge denomination. The extra amount
    // is used for fees to create deposit transaction
    if addr_txn.value.to_sat() <= config.deposit_amount.to_sat() {
        return None;
    }

    // Construct and return the DepositRequestInfo
    Some(DepositRequestInfo {
        amt: addr_txn.value.to_sat(),
        address: ee_bytes,
        take_back_leaf_hash: tap_ctrl_blk_hash,
    })
}

/// extracts the tapscript block and EE address given that the script is OP_RETURN type and
/// contains the Magic Bytes
fn parse_deposit_request_script(
    script: &ScriptBuf,
    config: &DepositTxParams,
) -> Option<DepositRequestScriptInfo> {
    let mut instructions = script.instructions();

    // check if OP_RETURN is present and if not just discard it
    if next_op(&mut instructions) != Some(OP_RETURN) {
        return None;
    }

    let data = next_bytes(&mut instructions)?;

    // Added a cfg to assert since it feels like it could crash us in
    // production.  I believe this is just a tx standardness policy, not a
    // consensus rule.
    //
    // Since it's not a consensus rule, it's a normal error not a panic.
    #[cfg(debug_assertions)]
    if data.len() > 80 {
        return None;
    }

    parse_tag_script(data, config)
}

fn parse_tag_script(
    buf: &[u8],
    config: &DepositTxParams,
) -> Option<DepositRequestScriptInfo> {
    // buf has expected magic bytes
    let magic_bytes = &config.magic_bytes;
    let magic_len = magic_bytes.len();
    if buf.len() < magic_len {
        return None;
    }
    let actual_magic_bytes = &buf[..magic_len];
    if actual_magic_bytes != magic_bytes {
        return None;
    }

    // 32 bytes of control hash
    let buf = &buf[magic_len..];
    if buf.len() < 32 {
        return None;
    }
    let ctrl_hash: &[u8; 32] = buf[..32].try_into().ok()?;

    // configured bytes for address
    let dest = &buf[32..];
    if dest.len() != config.max_address_length as usize {
        debug!(
            buf = ?buf,
            dest = ?dest,
            expected = config.max_address_length,
            got = %dest.len(),
            "incorrect number of bytes in dest buf"
        );
        return None;
    }

    Some(DepositRequestScriptInfo {
        tap_ctrl_blk_hash: *ctrl_hash,
        ee_bytes: dest.into(),
    })
}

/// Extract next instruction and try to parse it as an opcode
fn next_op(instructions: &mut Instructions<'_>) -> Option<Opcode> {
    let nxt = instructions.next();
    match nxt {
        Some(Ok(Instruction::Op(op))) => Some(op),
        _ => None,
    }
}

/// Extract next instruction and try to parse it as a byte slice
fn next_bytes<'a>(instructions: &mut Instructions<'a>) -> Option<&'a [u8]> {
    let ins = instructions.next();
    match ins {
        Some(Ok(Instruction::PushBytes(bytes))) => Some(bytes.as_bytes()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bdk_wallet::bitcoin::{absolute::LockTime, Amount, Transaction};

    use super::extract_deposit_request_info;
    use crate::constants::MAGIC_BYTES;
    use strata_primitives::l1::BitcoinAmount;
    use strata_params::DepositTxParams;

    #[test]
    fn test_empty_transaction() {
        let config = DepositTxParams {
            magic_bytes: *MAGIC_BYTES,
            max_address_length: 20,
            deposit_amount: BitcoinAmount::from_sat(1000000),
            address: Default::default(),
            operators_pubkey: Default::default(),
        };

        // Empty transaction with no outputs
        let test_transaction = Transaction {
            version: bitcoin::transaction::Version(2),
            lock_time: LockTime::ZERO,
            input: vec![],
            output: vec![],
        };

        let out = extract_deposit_request_info(&test_transaction, &config);

        // Should return None as the transaction has no outputs
        assert!(out.is_none());
    }
}
