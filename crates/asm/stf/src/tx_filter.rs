// TODO: Move the logic with other libraries to do all the L1 transaction parsing logic.
use std::collections::BTreeMap;

use bitcoin::{Transaction, opcodes::all::OP_RETURN, script::Instruction};
use strata_asm_common::{SubprotocolId, TagPayload, TxInput};

/// Attempt to parse the SPS-50 L1 transaction header from the first output of a Bitcoin
/// `Transaction`.
///
/// The SPS-50 header MUST be encoded as an `OP_RETURN` in output index 0, with payload:
/// ```text
/// [0..4]   ASCII magic "ALPN"
/// [4]      subprotocol type (u8)
/// [5]      tx type (u8)
/// [6..]    auxiliary data (ignored here)
/// ```
fn parse_sps50_header(tx: &Transaction) -> Option<(SubprotocolId, TagPayload<'_>)> {
    // 1) Ensure there's an output 0
    let first_out = tx.output.first()?;
    let script = &first_out.script_pubkey;

    // 2) Iterate instructions: expect first to be the OP_RETURN opcode
    let mut instrs = script.instructions();
    match instrs.next()? {
        Ok(Instruction::Op(op)) if op == OP_RETURN => {}
        _ => return None,
    }

    // 3) Next instruction must push the header bytes (>= 6 bytes)
    let data = match instrs.next()? {
        Ok(Instruction::PushBytes(d)) if d.len() >= 6 => d,
        _ => return None,
    };

    // 4) Verify magic "ALPN"
    if !data.as_bytes().starts_with(b"ALPN") {
        return None;
    }

    // 4) Extract subprotocol and tx type
    let subprotocol = data[4];

    let sps_50_payload = TagPayload::new(data[5], data[5..].as_bytes());
    Some((subprotocol, sps_50_payload))
}

/// Groups only those Bitcoin `Transaction`s tagged with an SPS-50 header,
/// keyed by their subprotocol type.
///
/// Transactions that lack a valid SPS-50 header (wrong magic, not OP_RETURN in
/// output[0], or too-short payload) are filtered out.
/// Returns references to the original transactions wrapped in `TxInput`.
pub(crate) fn group_txs_by_subprotocol<'t, I>(
    transactions: I,
) -> BTreeMap<SubprotocolId, Vec<TxInput<'t>>>
where
    I: IntoIterator<Item = &'t Transaction>,
{
    let mut map: BTreeMap<SubprotocolId, Vec<TxInput<'t>>> = BTreeMap::new();

    for tx in transactions {
        if let Some((subp, payload)) = parse_sps50_header(tx) {
            map.entry(subp).or_default().push(TxInput::new(tx, payload));
        }
    }

    map
}
