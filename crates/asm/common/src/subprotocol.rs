//! Subprotocol trait definition for ASM.
//!
//! This trait defines the interface every ASM subprotocol implementation must
//! provide. Each subprotocol is responsible for parsing its transactions,
//! updating its internal state, and emitting cross-protocol messages and logs.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
pub use strata_l1_txfmt::SubprotocolId;

use crate::{
    AnchorState, AsmError, AuxRequestCollector, SectionState, TxInputRef, VerifiedAuxData,
    log::AsmLogEntry, msg::InterprotoMsg,
};

/// Trait for defining subprotocol behavior within the ASM framework.
///
/// Subprotocols are modular components that can be plugged into the ASM to handle
/// specific transaction types and maintain their own state within the anchor state.
/// Each subprotocol defines its own transaction processing logic, message handling,
/// and state management.
///
///
/// 1. processes each new L1 block to update its own state and emit outgoing inter-protocol
///    messages, and then
/// 2. receives incoming messages to finalize and serialize its state for inclusion in the global
///    AnchorState.
///
/// # Example
///
/// ```ignore
/// struct MySubprotocol;
///
/// impl Subprotocol for MySubprotocol {
///     const ID: SubprotocolId = 42;
///     type State = MyState;
///     type Params = MyParams;
///     type Msg = MyMessage;
///
///     fn init(params: &Self::Params) -> Result<Self::State, AsmError> {
///        // init logic
///     }
///
///     fn pre_process_txs(
///         state: &Self::State,
///         txs: &[TxInputRef],
///         collector: &mut AuxRequestCollector,
///         anchor_pre: &AnchorState,
///         params: &Self::Params,
///     ) {
///         // Pre-process transactions and request auxiliary data
///     }
///
///     fn process_txs(
///         state: &mut Self::State,
///         txs: &[TxInputRef],
///         anchor_pre: &AnchorState,
///         verified_aux_data: &VerifiedAuxData,
///         relayer: &mut impl MsgRelayer,
///         params: &Self::Params,
///     ) {
///         // Process transactions
///     }
///
///     fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], params: &Self::Params) {
///         // Process messages
///     }
/// }
/// ```
pub trait Subprotocol: 'static {
    /// The subprotocol ID used when searching for relevant transactions.
    const ID: SubprotocolId;

    /// Type that defines the params the subprotocol should operate under, which
    /// might change dependent on block height.
    type Params;

    /// State type serialized into the ASM state structure.
    type State: Any + BorshDeserialize + BorshSerialize;

    /// Message type that we receive messages from other subprotocols using.
    type Msg: Clone + InterprotoMsg + Any;

    /// Constructs a new state using the provided genesis configuration.
    ///
    /// # Arguments
    /// * `params` - The subprotocol's params, from which we should be able to derive an initial
    ///   state to use when the pre-state does not contain an instance.
    ///
    /// # Returns
    ///
    /// The initialized state or an error if initialization fails.
    // FIXME why would this ever error?  this would be panic-worthy
    fn init(params: &Self::Params) -> Result<Self::State, AsmError>;

    /// Pre-processes a batch of L1 transactions by registering any required auxiliary data.
    ///
    /// During this phase, the subprotocol declares *external* data it will need before actual
    /// processing. Any required L1 headers, block-metadata, or other off-chain data should be
    /// requested via the `AuxRequestCollector`.
    /// (e.g., Merkle proof for logs emitted in a previous block from "history_mmr" in AnchorState)
    ///
    /// This method is called before transaction processing to allow subprotocols to specify
    /// any auxiliary data they need (such as L1 block headers, Merkle proofs, or other metadata).
    /// The requested data will be made available during the subsequent `process_txs` call.
    ///
    /// # Arguments
    /// * `state` - Current state of the subprotocol
    /// * `txs` - Slice of L1 transactions relevant to this subprotocol
    /// * `collector` - Interface for registering auxiliary input requirements
    /// * `anchor_pre` - The previous anchor state for context
    /// * `params` - Subprotocol's current params
    fn pre_process_txs(
        _state: &Self::State,
        _txs: &[TxInputRef<'_>],
        _collector: &mut AuxRequestCollector,
        _anchor_pre: &AnchorState,
        _params: &Self::Params,
    ) {
        // default nothing
    }

    /// Processes a batch of L1 transactions, extracting all relevant information for this
    /// subprotocol.
    ///
    /// This is the core transaction processing method where subprotocols implement their
    /// specific business logic. The method receives auxiliary data (requested
    /// during `pre_process_txs`) and can generate messages to other subprotocols and emit logs.
    ///
    /// # Arguments
    /// * `state` - Mutable reference to the subprotocol's state
    /// * `txs` - Slice of L1 transactions relevant to this subprotocol
    /// * `anchor_pre` - The previous anchor state for validation context
    /// * `verified_aux_data` - Verified auxiliary data previously requested and validated
    /// * `relayer` - Interface for sending messages to other subprotocols and emitting logs
    /// * `params` - Subprotocol's current params
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        verified_aux_data: &VerifiedAuxData,
        relayer: &mut impl MsgRelayer,
        params: &Self::Params,
    );

    /// Processes messages received from other subprotocols.
    ///
    /// This method handles inter-subprotocol communication, allowing subprotocols to
    /// react to events and data from other components in the ASM.
    ///
    /// # Arguments
    /// * `state` - Mutable reference to the subprotocol's state
    /// * `msgs` - Slice of messages received from other subprotocols
    /// * `params` - Subprotocol's current params
    ///
    /// TODO:
    /// Also generate the event logs that is later needed for other components
    /// to read ASM activity. Return the commitment of the events. The actual
    /// event is defined by the subprotocol and is not visible to the ASM.
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg], params: &Self::Params);
}

/// Generic message relayer interface which subprotocols can use to interact
/// with each other and the outside world.
pub trait MsgRelayer: Any {
    /// Relays a message to the destination subprotocol.
    fn relay_msg(&mut self, m: &dyn InterprotoMsg);

    /// Emits an output log message.
    fn emit_log(&mut self, log: AsmLogEntry);

    /// Gets this msg relayer as a `&dyn Any`.
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

/// Subprotocol handler trait for a loaded subprotocol.
pub trait SubprotoHandler {
    /// Gets the ID of the subprotocol.  This should just directly expose it
    /// as-is.
    fn id(&self) -> SubprotocolId;

    /// Pre-processes a batch of L1 transactions by delegating to the inner
    /// subprotocol's `pre_process_txs` implementation.
    ///
    /// Any required auxiliary data should be registered via the provided `AuxRequestCollector` for
    /// the subsequent processing phase.
    fn pre_process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        collector: &mut AuxRequestCollector,
        anchor_state: &AnchorState,
    );

    /// Processes a batch of L1 transactions by delegating to the underlying subprotocol's
    /// `process_txs` implementation.
    ///
    /// Messages and logs generated by the subprotocol will be sent via the provided `MsgRelayer`.
    fn process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        relayer: &mut dyn MsgRelayer,
        anchor_state: &AnchorState,
        verified_aux_data: &VerifiedAuxData,
    );

    /// Accepts a message.  This is called while processing other subprotocols.
    /// These should not be processed until we do the finalization.
    ///
    /// This MUST NOT act on any messages that were accepted before this was
    /// called.
    ///
    /// # Panics
    ///
    /// If an mismatched message type (behind the `dyn`) is provided.
    fn accept_msg(&mut self, msg: &dyn InterprotoMsg);

    /// Processes the buffered messages stored in the handler.
    fn process_buffered_msgs(&mut self);

    /// Repacks the state into a [`SectionState`] instance.
    fn to_section(&self) -> SectionState;
}
