//! Operations shared by the rule sets after V1 that run the V1 code within
//! their own capacities.
//!
//! V2 and V3 run every operation with the [`strata_ol_stf_v1`] code. Around
//! it they check their transaction effect and block log capacities, and the
//! operations that open a block or replay an epoch take over the state of the
//! spec before them at the first block of an epoch. [`CappedV1Rules`]
//! implements those operations once, for one spec, its predecessor, and its
//! capacities. Each rule set's [`OLStfSpec`](crate::spec::OLStfSpec)
//! implementation calls them one operation at a time, so a rule set that
//! changes an operation replaces only its own call and leaves the other rule
//! sets as they are.
//!
//! # Taking over the predecessor's state
//!
//! The epoch after an epoch that processed a checkpoint predicate enactment
//! runs the next spec. No rule set stages a spec, so that epoch starts from a
//! terminal state with `cur = staged` at the predecessor. Its first block
//! records the spec as `cur = staged`, which the state root commits from that
//! block on, and which node-side spec selection reads as the epoch's spec.
//!
//! Each operation that opens a block or replays an epoch takes over the state
//! before it runs the V1 code, so every driver, DA replay included, reaches
//! the same root. The V1 code then leaves the versions as they are: its
//! epoch-initial processing wraps only a V0 state, and it rejects only a V0
//! state outside an epoch start. Taking over before the V1 code checks the
//! block changes nothing on success, and callers discard the state of a failed
//! operation.
//!
//! The operations that open a block or replay an epoch take over nothing else.
//! They fail, without touching the state, on a predecessor state outside an
//! epoch start, on the state of any earlier spec, and on the state of a later
//! spec: each enactment advances the spec by one, so only a wrong spec
//! selection reaches any of these. The other operations run the V1 code
//! without checking the versions. The transaction, manifest, and
//! epoch-terminal phases run inside a block that
//! [`OLStfSpec::execute_block_initialization`](crate::spec::OLStfSpec::execute_block_initialization)
//! opened, and mempool admission checks a transaction for the first block of a
//! spec against the predecessor state that block takes over.
//!
//! # Capacities
//!
//! The V1 types keep their larger bounds, so V1 txids and encodings never
//! change, and the capacities are checked around the V1 code instead. Each
//! operation that executes a transaction checks its effects before the V1
//! code runs it, and each operation that executes a whole block checks the
//! block's logs after the V1 code emits them. A transaction or block above a
//! capacity is invalid. Block assembly composes the
//! [`sequencer`](crate::sequencer) phases itself, so it checks the block log
//! capacity on its own.

use std::slice;

use strata_acct_types::{AccountId, TxEffects};
use strata_codec::CodecError;
use strata_ol_chain_types_v1::{
    AsmManifest, OLBlockBodyV1, OLBlockHeaderV1, OLBlockV1, OLLog, OLTxSegmentV1,
};
use strata_ol_da_types_v1::{
    OLDaPayloadV1, OLDaSchemeV1, decode_ol_da_payload_bytes, decode_ol_da_payload_bytes_canonical,
};
use strata_ol_params::{BridgeParams, OLRuntimeParams};
use strata_ol_state_types::{
    IAccountState, IStateAccessorMut, OLSpecId, OLSpecVersions, TxProofIndexer,
};
use strata_ol_stf_v1::{
    self as v1, BasicExecContext, BlockComponents, BlockContext, ConstructBlockOutput,
    EpochExecExpectations, EpochInfo, ExecError, ExecResult, RuleAmendments, TxExecContext,
};
use strata_ol_tx_types_v1::{OLTransactionV1, SauTxOperationDataV1};

use crate::da::EpochDaReplayError;
use crate::limits::OLSpecLimits;

/// The rules of a spec after V1 that runs the V1 code within its own
/// capacities and takes over the state of the spec before it.
///
/// `predecessor` is the spec whose state `spec` takes over, and comes before
/// it in activation order. `amendments` are the rules the spec amends in the
/// V1 code, and `da_encoding` is the encoding of the epoch DA diff it
/// replays.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CappedV1Rules {
    spec: OLSpecId,
    predecessor: OLSpecId,
    limits: OLSpecLimits,
    amendments: RuleAmendments,
    da_encoding: DaDiffEncoding,
}

impl CappedV1Rules {
    /// Creates the rules of `spec`, which takes over `predecessor`'s state,
    /// allows `limits`, runs the V1 code with `amendments`, and replays epoch
    /// DA diffs in `da_encoding`.
    pub(crate) const fn new(
        spec: OLSpecId,
        predecessor: OLSpecId,
        limits: OLSpecLimits,
        amendments: RuleAmendments,
        da_encoding: DaDiffEncoding,
    ) -> Self {
        Self {
            spec,
            predecessor,
            limits,
            amendments,
            da_encoding,
        }
    }

    /// Returns the transaction effect and block log capacities these rules
    /// allow.
    pub(crate) fn limits(&self) -> OLSpecLimits {
        self.limits
    }

    /// Verifies a block: takes over the state at an epoch start, checks each
    /// transaction's effects, runs the V1 code, and checks the block's logs.
    pub(crate) fn verify_block<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        header: &OLBlockHeaderV1,
        parent_header: Option<&OLBlockHeaderV1>,
        body: &OLBlockBodyV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        self.take_over_state(state, opens_epoch(parent_header))?;
        if let Some(tx_segment) = body.tx_segment() {
            self.check_tx_segment_effects(tx_segment)?;
        }
        let logs = v1::verify_block_with_amendments(
            state,
            header,
            parent_header,
            body,
            runtime_params,
            self.amendments,
        )?;
        self.limits.check_block_log_count(logs.len())?;
        Ok(logs)
    }

    /// Constructs a block: takes over the state at an epoch start, checks each
    /// transaction's effects, runs the V1 code, and checks the block's logs.
    pub(crate) fn construct_block<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        block_context: BlockContext<'_>,
        block_components: BlockComponents,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<ConstructBlockOutput> {
        self.take_over_state(state, block_context.is_epoch_initial())?;
        self.check_tx_segment_effects(block_components.tx_segment())?;
        let output = v1::construct_block_with_amendments(
            state,
            block_context,
            block_components,
            runtime_params,
            self.amendments,
        )?;
        self.limits
            .check_block_log_count(output.outputs().logs().len())?;
        Ok(output)
    }

    /// Executes a batch of blocks without the epoch-terminal drain, checking
    /// each block as [`Self::verify_block`] does.
    pub(crate) fn execute_block_batch_predrain<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        blocks: &[OLBlockV1],
        initial_parent: &OLBlockHeaderV1,
        runtime_params: &OLRuntimeParams,
    ) -> ExecResult<Vec<OLLog>> {
        // Without the epoch-terminal drain the state stays in the epoch of the
        // first block, so only that block can open an epoch.
        if !blocks.is_empty() {
            self.take_over_state(state, opens_epoch(Some(initial_parent)))?;
        }
        // The V1 operation concatenates the logs of all blocks, so this runs
        // it one block at a time to count each block's logs.
        let mut batch_logs = Vec::new();
        let mut parent = initial_parent;
        for block in blocks {
            if let Some(tx_segment) = block.body().tx_segment() {
                self.check_tx_segment_effects(tx_segment)?;
            }
            let logs = v1::execute_block_batch_predrain_with_amendments(
                state,
                slice::from_ref(block),
                parent,
                runtime_params,
                self.amendments,
            )?;
            self.limits.check_block_log_count(logs.len())?;
            batch_logs.extend(logs);
            parent = block.header();
        }
        Ok(batch_logs)
    }

    /// Replays an epoch from its encoded V1 DA diff after taking over the
    /// state.
    pub(crate) fn apply_da_epoch<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        // An epoch replay always opens the epoch. Taking over first checks the
        // state before decoding, so a rejected replay never depends on the
        // diff.
        self.take_over_state(state, true)?;
        let diff = self
            .da_encoding
            .decode(encoded_diff)
            .map_err(EpochDaReplayError::Decode)?;
        v1::apply_da_epoch::<S, OLDaSchemeV1>(state, epoch_info, diff, manifests, runtime_params)
            .map_err(EpochDaReplayError::Exec)
    }

    /// Replays an epoch from its encoded V1 DA diff after taking over the
    /// state, and checks the result against `exp`.
    pub(crate) fn verify_epoch_with_diff<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        epoch_info: &EpochInfo,
        encoded_diff: &[u8],
        manifests: &[AsmManifest],
        exp: &EpochExecExpectations,
        runtime_params: &OLRuntimeParams,
    ) -> Result<(), EpochDaReplayError> {
        self.take_over_state(state, true)?;
        let diff = self
            .da_encoding
            .decode(encoded_diff)
            .map_err(EpochDaReplayError::Decode)?;
        v1::verify_epoch_with_diff::<S, OLDaSchemeV1>(
            state,
            epoch_info,
            diff,
            manifests,
            exp,
            runtime_params,
        )
        .map_err(EpochDaReplayError::Exec)
    }

    /// Opens a block after taking over the state at an epoch start.
    pub(crate) fn execute_block_initialization<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        block_context: &BlockContext<'_>,
    ) -> ExecResult<()> {
        self.take_over_state(state, block_context.is_epoch_initial())?;
        v1::execute_block_initialization(state, block_context)
    }

    /// Checks a transaction's effects, then processes it with the V1 code.
    pub(crate) fn process_single_tx<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        tx: &OLTransactionV1,
        context: &TxExecContext<'_>,
    ) -> ExecResult<()> {
        self.limits.check_tx_effects(tx.data().effects())?;
        // The caller builds the context without knowing the amendments, so
        // this selects them for the same block and output buffer.
        let basic_context = context.basic_context().with_amendments(self.amendments);
        let context = TxExecContext::new(&basic_context, context.parent_header());
        v1::process_single_tx(state, tx, &context)
    }

    /// Processes the epoch terminal with the V1 code under these rules'
    /// amendments.
    pub(crate) fn process_epoch_terminal<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        context: &BasicExecContext<'_>,
    ) -> ExecResult<()> {
        // As in `process_single_tx`, the caller's context does not know the
        // amendments.
        v1::process_epoch_terminal(state, &context.with_amendments(self.amendments))
    }

    /// Predicts the log payloads of a successful transaction under these
    /// rules' amendments.
    pub(crate) fn predict_tx_log_payloads(
        &self,
        tx: &OLTransactionV1,
        bridge_params: &BridgeParams,
        on_payload: impl FnMut(&[u8]),
    ) -> ExecResult<()> {
        v1::predict_tx_log_payloads(tx, bridge_params, self.amendments, on_payload)
            .map_err(Into::into)
    }

    /// Checks a snark account update's effects, then lists the proofs it
    /// needs.
    pub(crate) fn index_snark_update_proof_requirements(
        &self,
        target: AccountId,
        account_state: &impl IAccountState,
        sau_op: &SauTxOperationDataV1,
        effects: &TxEffects,
    ) -> ExecResult<TxProofIndexer> {
        self.limits.check_tx_effects(effects)?;
        // The indexer records each proof check instead of verifying it, so the
        // update's proof checks run as a dry run that lists what it needs.
        let mut proof_indexer = TxProofIndexer::new_fresh();
        v1::verify_snark_acct_update_proofs(
            target,
            account_state,
            sau_op,
            effects,
            &mut proof_indexer,
        )?;
        Ok(proof_indexer)
    }

    /// Checks the effects of each transaction in a block against the
    /// capacities.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::TxEffectsOverflow`] for the first transaction above a
    /// capacity, wrapped with its txid and index as block execution wraps
    /// transaction errors.
    fn check_tx_segment_effects(&self, tx_segment: &OLTxSegmentV1) -> ExecResult<()> {
        for (index, tx) in tx_segment.txs().iter().enumerate() {
            self.limits
                .check_tx_effects(tx.data().effects())
                .map_err(|err| ExecError::from(err).with_tx(tx.compute_txid(), index))?;
        }
        Ok(())
    }

    /// Takes over a predecessor state at the first block of an epoch, and
    /// checks that any other work runs on a state of this spec.
    ///
    /// `opens_epoch` says whether the work starts an epoch. A state of this
    /// spec is left as it is.
    ///
    /// # Errors
    ///
    /// Returns [`ExecError::ContinuesEarlierSpecEpoch`] for a predecessor state
    /// outside an epoch start, [`ExecError::StateFromEarlierSpec`] for the
    /// state of a spec before the predecessor, and
    /// [`ExecError::StateFromLaterSpec`] for the state of a later spec. None of
    /// them touches the state.
    fn take_over_state<S: IStateAccessorMut>(
        &self,
        state: &mut S,
        opens_epoch: bool,
    ) -> ExecResult<()> {
        let state_spec = state.spec_versions().cur_spec();
        if state_spec == self.spec {
            Ok(())
        } else if state_spec == self.predecessor {
            if !opens_epoch {
                return Err(ExecError::ContinuesEarlierSpecEpoch {
                    spec: self.spec,
                    state_spec,
                });
            }
            state.set_spec_versions(OLSpecVersions::uniform(self.spec));
            Ok(())
        } else if state_spec < self.predecessor {
            Err(ExecError::StateFromEarlierSpec {
                spec: self.spec,
                state_spec,
            })
        } else {
            Err(ExecError::StateFromLaterSpec {
                spec: self.spec,
                state_spec_version: state_spec.into(),
            })
        }
    }
}

/// The encodings of an epoch's V1 DA diff a rule set replays.
///
/// The checkpoint claim commits to the diff's bytes, not to the diff they
/// decode to, so the rules also decide which bytes spell a diff.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DaDiffEncoding {
    /// Every encoding the V1 DA decoders read, including the ones the encoder
    /// never writes.
    ///
    /// The V1 and V2 checkpoint programs read all of them, so a node has to
    /// replay each one to follow the epochs those programs prove.
    AnyDecodable,

    /// Only the encoding the encoder writes, so a diff has one spelling.
    Canonical,
}

impl DaDiffEncoding {
    /// Decodes an encoded DA diff, accepting the encodings this names.
    fn decode(self, encoded_diff: &[u8]) -> Result<OLDaPayloadV1, CodecError> {
        match self {
            Self::AnyDecodable => decode_ol_da_payload_bytes(encoded_diff),
            Self::Canonical => decode_ol_da_payload_bytes_canonical(encoded_diff),
        }
    }
}

/// Returns whether the block after `parent_header` is the first block of an
/// epoch, as [`BlockContext::is_epoch_initial`] decides it.
fn opens_epoch(parent_header: Option<&OLBlockHeaderV1>) -> bool {
    parent_header.is_none_or(OLBlockHeaderV1::is_terminal)
}
