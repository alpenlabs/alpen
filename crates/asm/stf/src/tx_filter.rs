// TODO: Move the logic with other libraries to do all the L1 transaction parsing logic.
use std::collections::BTreeMap;

use bitcoin::Transaction;
use strata_asm_common::{SubprotocolId, TxInput};
use strata_l1_txfmt::{MagicBytes, ParseConfig};

/// Groups only those Bitcoin `Transaction`s tagged with an SPS-50 header,
/// keyed by their subprotocol type.
///
/// Transactions that lack a valid SPS-50 header (wrong magic, not OP_RETURN in
/// output[0], or too-short payload) are filtered out.
/// Returns references to the original transactions wrapped in `TxInput`.
pub(crate) fn group_txs_by_subprotocol<'t, I>(
    magic: MagicBytes,
    transactions: I,
) -> BTreeMap<SubprotocolId, Vec<TxInput<'t>>>
where
    I: IntoIterator<Item = &'t Transaction>,
{
    let parser = ParseConfig::new(magic);
    let mut map: BTreeMap<SubprotocolId, Vec<TxInput<'t>>> = BTreeMap::new();

    for tx in transactions {
        if let Ok(payload) = parser.try_parse_tx(tx) {
            map.entry(payload.subproto_id())
                .or_default()
                .push(TxInput::new(tx, payload));
        }
    }

    map
}
