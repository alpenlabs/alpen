use revm::{
    context::{
        result::{EVMError, ExecResultAndState, ExecutionResult, HaltReason, InvalidTransaction},
        ContextSetters, ContextTr, JournalTr,
    },
    handler::{instructions::EthInstructions, EvmTr, Handler},
    inspector::{InspectorHandler, JournalExt},
    interpreter::interpreter::EthInterpreter,
    state::EvmState,
    Database, DatabaseCommit, ExecuteCommitEvm, ExecuteEvm, InspectCommitEvm, InspectEvm,
    Inspector,
};

use crate::api::{evm::AlpenEvmInner, handler::AlpenRevmHandler};

/// Type alias for the error type of the AlpenEvm.
type AlpenEvmError<CTX> = EVMError<<<CTX as ContextTr>::Db as Database>::Error, InvalidTransaction>;

// Trait that allows to replay and transact the transaction.
impl<CTX, INSP> ExecuteEvm for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>>
where
    CTX: ContextSetters<Journal: JournalTr<State = EvmState>>,
{
    type State = EvmState;
    type ExecutionResult = ExecutionResult<HaltReason>;
    type Error = AlpenEvmError<CTX>;

    type Tx = <CTX as ContextTr>::Tx;

    type Block = <CTX as ContextTr>::Block;

    fn set_block(&mut self, block: Self::Block) {
        self.evm_ctx.ctx.set_block(block);
    }
    fn replay(
        &mut self,
    ) -> Result<ExecResultAndState<Self::ExecutionResult, Self::State>, Self::Error> {
        let mut handler = AlpenRevmHandler::default();
        handler.run(self).map(|result| {
            let state = self.finalize();
            ExecResultAndState::new(result, state)
        })
    }

    fn transact_one(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.evm_ctx.ctx.set_tx(tx);
        let mut handler = AlpenRevmHandler::default();
        handler.run(self)
    }

    fn finalize(&mut self) -> Self::State {
        self.ctx().journal_mut().finalize()
    }
}

// Trait allows replay_commit and transact_commit functionality.
impl<CTX, INSP> ExecuteCommitEvm for AlpenEvmInner<CTX, INSP>
where
    CTX: ContextSetters<Db: DatabaseCommit, Journal: JournalTr<State = EvmState>>,
{
    fn commit(&mut self, state: Self::State) {
        self.ctx().db_mut().commit(state);
    }
}

impl<CTX, INSP> InspectEvm for AlpenEvmInner<CTX, INSP>
where
    CTX: ContextSetters<Journal: JournalTr<State = EvmState> + JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
    type Inspector = INSP;

    fn set_inspector(&mut self, inspector: Self::Inspector) {
        self.evm_ctx.inspector = inspector;
    }

    fn inspect_one_tx(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.evm_ctx.ctx.set_tx(tx);
        let mut handler = AlpenRevmHandler::default();
        handler.inspect_run(self)
    }
}

// Inspect
impl<CTX, INSP> InspectCommitEvm for AlpenEvmInner<CTX, INSP>
where
    CTX: ContextSetters<Db: DatabaseCommit, Journal: JournalTr<State = EvmState> + JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
}
