use revm::{
    context::{
        result::{EVMError, ExecutionResult, ResultAndState},
        ContextSetters, ContextTr, JournalOutput, JournalTr,
    },
    handler::{instructions::EthInstructions, EthFrame, PrecompileProvider},
    inspector::JournalExt,
    interpreter::{interpreter::EthInterpreter, InterpreterResult},
    DatabaseCommit, ExecuteCommitEvm, ExecuteEvm, InspectCommitEvm, InspectEvm, Inspector,
};

use crate::api::{evm::AlpenEvmInner, handler::AlpenRevmHandler};

impl<CTX, INSP, PRECOMPILE> ExecuteEvm
    for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: ContextSetters<Journal: JournalTr<FinalOutput = JournalOutput>>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type Output = Result<ResultAndState, EVMError<CTX>>;

    type Tx = <CTX as ContextTr>::Tx;

    type Block = <CTX as ContextTr>::Block;

    fn set_tx(&mut self, tx: Self::Tx) {
        self.0.data.ctx.set_tx(tx);
    }

    fn set_block(&mut self, block: Self::Block) {
        self.0.data.ctx.set_block(block);
    }

    fn replay(&mut self) -> Self::Output {
        // let mut h = AlpenRevmHandler::<_, _, EthFrame<_, _, _>>::new();
        // h.run(self)
        todo!()
    }
}

impl<CTX, INSP, PRECOMPILE> ExecuteCommitEvm
    for AlpenEvmInner<CTX, INSP, EthInstructions<EthInterpreter, CTX>, PRECOMPILE>
where
    CTX: ContextSetters<Db: DatabaseCommit, Journal: JournalTr<FinalOutput = JournalOutput>>,
    PRECOMPILE: PrecompileProvider<CTX, Output = InterpreterResult>,
{
    type CommitOutput = Result<ExecutionResult, EVMError<CTX>>;

    fn replay_commit(&mut self) -> Self::CommitOutput {
        self.replay().map(|r| {
            // self.ctx().db().commit(r.state);
            // r.result
            todo!()
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
        self.0.data.inspector = inspector;
    }

    fn inspect_replay(&mut self) -> Self::Output {
        // let mut h = AlpenRevmHandler::<_, _, EthFrame<_, _, _>>::new();
        // h.inspect_run(self)
        todo!()
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
            // self.ctx().db().commit(r.state);
            // r.result
            todo!()
        })
    }
}
