# ASM Auxiliary Data Helper Crate

This crate bundles the helper types that subprotocol authors use when they need
auxiliary data. It contains:

- `AuxRequestCollector` - handed to `Subprotocol::pre_process_txs` so a
  subprotocol can describe which aux data it will need before execution.
- `AuxRequestSpec` - ready-made request payloads for common needs such as
  historical ASM logs and bridge deposit request transactions.
- `AuxResponseEnvelope` - the data the asm worker feeds back into
  the ASM STF once it has satisfied those requests.
- `SubprotocolAuxResolver` - a wrapper that scopes all auxiliary responses to
  a single subprotocol, verifies inclusion proofs against the header MMR, and
  exposes helper methods to retrieve fully-verified data during `process_txs`.

The orchestration flow looks like this:

1. During preprocessing the ASM STF invokes `pre_process_txs`.  The
   subprotocol calls `collector.request_aux_input(tx.l1_tx_index(), ...)` for
   every transaction that needs an aux witness.
2. The orchestration layer serializes those requests, fetches the data (e.g.
   Merkle proofs for historical logs, or a deposit request transaction), and
   packs the results into a list of `AuxResponseEnvelope` values.
3. When `process_txs` runs, the STF passes a `SubprotocolAuxResolver` that is
   backed by those envelopes.  The subprotocol can look up responses by the
   original `l1_tx_index`.

## Worked Example

The following minimal subprotocol demonstrates both sides of the aux workflow.
It asks for historical logs when a checkpoint transaction appears, and then
validates the logs inside `process_txs`.

```rust
use strata_asm_aux::{AuxRequestCollector, AuxRequestSpec};
use strata_asm_common::{
    AnchorState, AsmError, AsmLogEntry, AuxInputCollector, AuxResolver, MsgRelayer, NullMsg,
    Subprotocol, SubprotocolId, TxInputRef,
};

const EXAMPLE_ID: SubprotocolId = SubprotocolId::from_u8(42);

struct ExampleParams;
struct ExampleState;

pub struct ExampleSubprotocol;

impl Subprotocol for ExampleSubprotocol {
    const ID: SubprotocolId = EXAMPLE_ID;
    type Params = ExampleParams;
    type State = ExampleState;
    type Msg = NullMsg<EXAMPLE_ID>;

    fn init(_: &Self::Params) -> Result<Self::State, AsmError> {
        Ok(ExampleState)
    }

    fn pre_process_txs(
        _state: &Self::State,
        txs: &[TxInputRef<'_>],
        collector: &mut impl AuxInputCollector,
        _anchor_pre: &AnchorState,
        _params: &Self::Params,
    ) {
        for tx in txs {
            // Suppose transaction tags contain the L1 block height they care about.
            let block_height = tx.tag().payload()[0] as u64;
            if tx.tag().tx_type() != SpecialTxTypeID {
                // Request all logs for that L1 height so we can prove inclusion.
                collector.request_aux_input(
                    tx.l1_tx_index(),
                    AuxRequestSpec::historical_logs(block_height).boxed(),
                );
            }
            ...
        }
    }

    fn process_txs(
        _state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        _anchor_pre: &AnchorState,
        aux_resolver: &dyn AuxResolver,
        _relayer: &mut impl MsgRelayer,
        _params: &Self::Params,
    ) {
        for tx in txs {
            if tx.tag().tx_type() != SpecialTxTypeID {
                let logs: Vec<AsmLogEntry> = aux_resolver
                    .historical_logs(tx.l1_tx_index())
                    .expect("aux: historical log lookup");
                
                // panic on error if logs are empty
                // continue processing with verified logs...
            }

            ...
        }
    }

    fn process_msgs(_: &mut Self::State, _: &[Self::Msg], _: &Self::Params) {}
}
```

When the requested data spans multiple consecutive L1 blocks, the orchestration
layer should pack the segments into the `HistoricalLogsRange` variant. The
resolver flattens those segments into the same `Vec<AsmLogEntry>` returned by
`historical_logs`, so subprotocol logic does not need to care whether the data
came from a single block or a range.

### Bridge Deposits

Bridge deposits can request their paired Deposit Request Transaction (DRT)
instead of embedding OP_RETURN data.  During preprocessing the bridge
subprotocol would call:

```rust
collector.request_aux_input(
    tx.l1_tx_index(),
    AuxRequestSpec::deposit_request_tx(drt_txid).boxed(),
);
```

Inside `process_txs` it would call
`aux_resolver.deposit_request_tx(tx.l1_tx_index())?`, deserialize the returned
raw transaction (if any), and then verify signatures and UTXO ownership before
continuing with state updates.

## Tips For Subprotocol Authors

- Always use `tx.l1_tx_index()` as the key when requesting aux data; the STF
  matches responses on that index.
- If you need different response payloads, define your own enum and make sure
  the orchestration layer stores those values in the aux response map.
- Prefer the provided `AuxRequestSpec` helpers for common requests; the enum
  makes it clear to the orchestration layer how to fulfil the data.
