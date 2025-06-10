use revm::{
    context::{
        result::{EVMError, ExecutionResult, InvalidTransaction, ResultAndState},
        ContextSetters, ContextTr, JournalOutput, JournalTr,
    },
    handler::{instructions::EthInstructions, EvmTr, Handler, PrecompileProvider},
    inspector::{InspectorHandler, JournalExt},
    interpreter::{interpreter::EthInterpreter, InterpreterResult},
    Database, DatabaseCommit, ExecuteCommitEvm, ExecuteEvm, InspectCommitEvm, InspectEvm,
    Inspector,
};

use crate::api::{evm::AlpenEvmInner, handler::AlpenRevmHandler};

/// Type alias for the error type of the AlpenEvm.
type AlpenEvmError<CTX> = EVMError<<<CTX as ContextTr>::Db as Database>::Error, InvalidTransaction>;

impl<CTX, INSP, PRECOMPILE> ExecuteEvm
    for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: ContextSetters<Journal: JournalTr<FinalOutput = JournalOutput>>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type Output = Result<ResultAndState, AlpenEvmError<CTX>>;

    type Tx = <CTX as ContextTr>::Tx;

    type Block = <CTX as ContextTr>::Block;

    fn set_tx(&mut self, tx: Self::Tx) {
        self.evm_ctx.data.ctx.set_tx(tx);
    }

    fn set_block(&mut self, block: Self::Block) {
        self.evm_ctx.data.ctx.set_block(block);
    }
    fn replay(&mut self) -> Self::Output {
        AlpenRevmHandler::default().run(self)
    }
}

impl<CTX, INSP, PRECOMPILE> ExecuteCommitEvm
    for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: ContextSetters<Db: DatabaseCommit, Journal: JournalTr<FinalOutput = JournalOutput>>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type CommitOutput = Result<ExecutionResult, AlpenEvmError<CTX>>;

    fn replay_commit(&mut self) -> Self::CommitOutput {
        self.replay().map(|r| {
            self.evm_ctx.ctx().db().commit(r.state);
            r.result
        })
    }
}

impl<CTX, INSP, PRECOMPILE> InspectEvm
    for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: ContextSetters<Journal: JournalTr<FinalOutput = JournalOutput> + JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type Inspector = INSP;

    fn set_inspector(&mut self, inspector: Self::Inspector) {
        self.evm_ctx.data.inspector = inspector;
    }

    fn inspect_replay(&mut self) -> Self::Output {
        AlpenRevmHandler::default().inspect_run(self)
    }
}

impl<CTX, INSP, PRECOMPILE> InspectCommitEvm
    for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: ContextSetters<
        Db: DatabaseCommit,
        Journal: JournalTr<FinalOutput = JournalOutput> + JournalExt,
    >,
    INSP: Inspector<CTX, EthInterpreter>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    fn inspect_replay_commit(&mut self) -> Self::CommitOutput {
        self.inspect_replay().map(|r| {
            self.evm_ctx.ctx().db().commit(r.state);
            r.result
        })
    }
}
