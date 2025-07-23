//! Subprotocol trait definition for ASM.
//!
//! This trait defines the interface every ASM subprotocol implementation must
//! provide. Each subprotocol is responsible for parsing its transactions,
//! updating its internal state, and emitting cross-protocol messages and logs.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
pub use strata_l1_txfmt::SubprotocolId;

use crate::{
    AnchorState, AsmError, AuxRequest, SectionState, TxInputRef, log::AsmLogEntry,
    msg::InterprotoMsg,
};

/// Trait for defining subprotocol behavior within the ASM framework.
///
/// Subprotocols are modular components that can be plugged into the ASM to handle
/// specific transaction types and maintain their own state within the anchor state.
/// Each subprotocol defines its own transaction processing logic, message handling,
/// and state management.
///
/// # Example
///
/// ```ignore
/// struct MySubprotocol;
///
/// impl Subprotocol for MySubprotocol {
///     const ID: SubprotocolId = 42;
///     type State = MyState;
///     type GenesisConfig = MyConfig;
///     type Msg = MyMessage;
///     type AuxInput = MyAuxData;
///
///     fn init(genesis_config_data: &[u8]) -> Result<Self::State, AsmError> {
///         let config: Self::GenesisConfig = borsh::from_slice(genesis_config_data)?;
///         Ok(MyState::from_config(config))
///     }
///
///     fn process_txs(state: &mut Self::State, txs: &[TxInputRef], ...) {
///         // Process transactions
///     }
/// }
/// ```
pub trait Subprotocol: 'static {
    /// The subprotocol ID used when searching for relevant transactions.
    const ID: SubprotocolId;

    /// State type serialized into the ASM state structure.
    type State: Any + BorshDeserialize + BorshSerialize;

    /// Message type that we receive messages from other subprotocols using.
    type Msg: Clone + Any;

    /// Type of auxiliary input required by the subprotocol.
    ///
    /// This associated type represents the exact data requested via `AuxInputCollector` (for
    /// example, block headers or other off-chain metadata). It must be serializable, verifiable,
    /// and correspond directly to the output of the collector. Implementations of
    /// `process_txs` are responsible for validating this data before using it in any state updates.
    type AuxInput: Any + BorshSerialize + BorshDeserialize;

    /// Genesis configuration type for initializing the subprotocol state.
    /// This should contain all necessary parameters for proper subprotocol initialization.
    type GenesisConfig: Any + BorshDeserialize + BorshSerialize;

    /// Constructs a new state using optional genesis configuration data.
    ///
    /// # Arguments
    /// * `genesis_config_data` - Optional serialized genesis configuration data that should be
    ///   deserialized into Self::GenesisConfig before use. Each subprotocol can decide whether to
    ///   accept None (empty genesis) or require proper configuration.
    ///
    /// # Returns
    /// The initialized state or an error if deserialization/initialization fails
    fn init(genesis_config_data: Option<&[u8]>) -> Result<Self::State, AsmError>;

    /// Pre-processes a batch of L1 transactions by registering any required off-chain inputs.
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
    fn pre_process_txs(
        state: &Self::State,
        txs: &[TxInputRef<'_>],
        collector: &mut impl AuxInputCollector,
        anchor_pre: &AnchorState,
    ) {
        // Default implementation: no auxiliary input required
        let _ = (state, txs, collector, anchor_pre);
    }

    /// Processes a batch of L1 transactions, extracting all relevant information for this
    /// subprotocol.
    ///
    /// This is the core transaction processing method where subprotocols implement their
    /// specific business logic. The method receives validated auxiliary inputs (requested
    /// during `pre_process_txs`) and can generate messages to other subprotocols and emit logs.
    ///
    /// # Arguments
    /// * `state` - Mutable reference to the subprotocol's state
    /// * `txs` - Slice of L1 transactions relevant to this subprotocol
    /// * `anchor_pre` - The previous anchor state for validation context
    /// * `aux_inputs` - Auxiliary data previously requested and validated
    /// * `relayer` - Interface for sending messages to other subprotocols and emitting logs
    fn process_txs(
        state: &mut Self::State,
        txs: &[TxInputRef<'_>],
        anchor_pre: &AnchorState,
        aux_inputs: &[Self::AuxInput],
        relayer: &mut impl MsgRelayer,
    );

    /// Processes messages received from other subprotocols.
    ///
    /// This method handles inter-subprotocol communication, allowing subprotocols to
    /// react to events and data from other components in the ASM.
    ///
    /// # Arguments
    /// * `state` - Mutable reference to the subprotocol's state
    /// * `msgs` - Slice of messages received from other subprotocols
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg]);
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
    /// Any required off-chain inputs should be registered via the provided `AuxInputCollector` for
    /// the subsequent processing phase.
    fn pre_process_txs(
        &mut self,
        txs: &[TxInputRef<'_>],
        collector: &mut dyn AuxInputCollector,
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

/// Responsible for recording a request for auxiliary input data.
///
/// The caller provides an opaque byte slice; the collector must interpret
/// those bytes out-of-band according to its own conventions
///
/// # Parameters
///
/// - `data`: an opaque byte slice whose meaning is defined entirely by the collector's
///   implementation.
///
/// # Panics
///
/// Implementations must understand the details of the subprotocol to understand the `data`
/// requested
pub trait AuxInputCollector: Any {
    /// Record that this exact `data` blob will be needed later as auxiliary input.
    fn request_aux_input(&mut self, req: AuxRequest);

    /// Gets this aux input collector as a `&dyn Any`.
    fn as_mut_any(&mut self) -> &mut dyn Any;
}
