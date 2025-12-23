use bitcoin::{
    ScriptBuf, XOnlyPublicKey,
    opcodes::all::{OP_CHECKSIGVERIFY, OP_CSV},
    script::Builder,
    secp256k1::SECP256K1,
    taproot::{TaprootBuilder, TaprootSpendInfo},
};

/// Creates the locking script for the Deposit Request Transaction (DRT) output.
///
/// This function constructs a P2TR (Pay-to-Taproot) locking script with two spending paths:
///
/// 1. **N/N multisig path (internal key)**: Allows the bridge operators to create a valid Deposit
///    Transaction by spending this output.
///
/// 2. **Recovery path (tapscript)**: Allows the depositor to reclaim their funds after a timeout
///    period if the bridge operators fail to process the deposit.
///
/// # Parameters
///
/// * `recovery_pk` - The depositor's x-only public key (32 bytes) for the recovery path. The
///   depositor can use the corresponding secret key to reclaim funds after the timeout if deposit
///   transaction is not created.
///
/// * `internal_key` - The N/N multisig aggregated public key for the cooperative spending path.
///   This represents the bridge operators' ability to create the Deposit Transaction.
///
/// * `recovery_delay` - The number of blocks that must pass before the depositor can use the
///   recovery path. This period should be sufficient for bridge operators to process the deposit
///   and the deposit request transaction is not spent.
///
/// # Returns
///
/// A `ScriptBuf` containing the P2TR locking script to be used in the DRT output.
///
/// # Panics
///
/// Panics if taproot finalization fails, which should not occur with valid inputs.
pub fn create_deposit_request_locking_script(
    recovery_pk: &[u8; 32],
    internal_key: XOnlyPublicKey,
    recovery_delay: u32,
) -> ScriptBuf {
    let spend_info = build_deposit_request_spend_info(recovery_pk, internal_key, recovery_delay);
    ScriptBuf::new_p2tr(SECP256K1, internal_key, spend_info.merkle_root())
}

/// Build the taproot spend info for the deposit request locking script.
pub fn build_deposit_request_spend_info(
    recovery_pk: &[u8; 32],
    internal_key: XOnlyPublicKey,
    recovery_delay: u32,
) -> TaprootSpendInfo {
    let tapscript = Builder::new()
        .push_slice(recovery_pk)
        .push_opcode(OP_CHECKSIGVERIFY)
        .push_int(recovery_delay as i64)
        .push_opcode(OP_CSV)
        .into_script();

    TaprootBuilder::new()
        .add_leaf(0, tapscript)
        .expect("valid tapscript leaf")
        .finalize(SECP256K1, internal_key)
        .expect("taproot finalization should succeed")
}
