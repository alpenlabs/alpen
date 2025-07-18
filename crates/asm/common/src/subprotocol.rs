//! Subprotocol trait definition for ASM.
//!
//! This trait defines the interface every ASM subprotocol implementation must
//! provide. Each subprotocol is responsible for parsing its transactions,
//! updating its internal state, and emitting cross-protocol messages and logs.

use std::any::Any;

use borsh::{BorshDeserialize, BorshSerialize};
pub use strata_l1_txfmt::SubprotocolId;

use crate::{Log, SectionState, TxInput, msg::InterprotoMsg};

/// ASM subprotocol interface.
///
/// A Subprotocol encapsulates a self-contained piece of logic that
///
/// 1. processes each new L1 block to update its own state and emit outgoing inter-protocol
///    messages, and then
/// 2. receives incoming messages to finalize and serialize its state for inclusion in the global
///    AnchorState.
pub trait Subprotocol: 'static {
    /// The subprotocol ID used when searching for relevant transactions.
    const ID: SubprotocolId;

    /// State type serialized into the ASM state structure.
    type State: Any + BorshDeserialize + BorshSerialize;

    /// Message type that we receive messages from other subprotocols using.
    type Msg: Clone + Any;

    /// Constructs a new state to use if the ASM does not have an instance of it.
    fn init() -> Self::State;

    /// Processes a batch of L1 transactions, extracting all relevant information for this
    /// subprotocol.
    ///
    /// Updates the subprotocol’s internal state and collects any resulting `InterprotoMsg` and
    /// `Log` on the provided `MsgRelayer`.
    fn process_txs(state: &mut Self::State, txs: &[TxInput<'_>], relayer: &mut impl MsgRelayer);

    /// Use the msgs other subprotocols to update its state.
    ///
    /// TODO:
    /// Also generate the event logs that is later needed for other components
    /// to read ASM activity. Return the commitment of the events. The actual
    /// event is defined by the subprotocol and is not visible to the ASM.
    fn process_msgs(state: &mut Self::State, msgs: &[Self::Msg]);
}

/// Generic message relayer interface which subprotocols can use to interact
/// with each other and the outside world.
pub trait MsgRelayer: Any {
    /// Relays a message to the destination subprotocol.
    fn relay_msg(&mut self, m: &dyn InterprotoMsg);

    /// Emits an output log message.
    fn emit_log(&mut self, log: Log);

    /// Gets this msg relayer as a `&dyn Any`.
    fn as_mut_any(&mut self) -> &mut dyn Any;
}

/// Subprotocol handler trait for a loaded subprotocol.
pub trait SubprotoHandler {
    /// Gets the ID of the subprotocol.  This should just directly expose it
    /// as-is.
    fn id(&self) -> SubprotocolId;

    /// Processes a batch of L1 transactions by delegating to the underlying subprotocol’s
    /// `process_txs` implementation.
    ///
    /// Messages and logs generated by the subprotocol will be sent via the provided `MsgRelayer`.
    fn process_txs(&mut self, txs: &[TxInput<'_>], relayer: &mut dyn MsgRelayer);

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
