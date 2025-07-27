use reth_evm::{eth::EthEvmContext, Database};
use revm::{
    context::{BlockEnv, ContextSetters, TxEnv},
    context_interface::{
        result::{EVMError, ExecutionResult, ResultAndState},
        ContextTr,
    },
    handler::Handler,
    inspector::{InspectCommitEvm, InspectEvm, Inspector, InspectorHandler},
    state::EvmState,
    DatabaseCommit, ExecuteCommitEvm, ExecuteEvm,
};

use crate::api::{handler::AlpenRevmHandler, AlpenEvm};

impl<DB, INSP> ExecuteEvm for AlpenEvm<DB, INSP>
where
    DB: Database,
{
    type ExecutionResult = ExecutionResult;
    type State = EvmState;
    type Error = EVMError<DB::Error>;
    type Tx = TxEnv;
    type Block = BlockEnv;

    fn set_block(&mut self, block: Self::Block) {
        self.inner.set_block(block);
    }

    fn transact_one(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.inner.ctx.set_tx(tx);
        AlpenRevmHandler::default().run(self)
    }

    fn finalize(&mut self) -> Self::State {
        self.inner.finalize()
    }

    fn replay(&mut self) -> Result<ResultAndState, Self::Error> {
        AlpenRevmHandler::default().run(self).map(|result| {
            let state = self.finalize();
            ResultAndState::new(result, state)
        })
    }
}

impl<DB, INSP> ExecuteCommitEvm for AlpenEvm<DB, INSP>
where
    DB: Database + DatabaseCommit,
{
    fn commit(&mut self, state: Self::State) {
        self.inner.ctx.db_mut().commit(state);
    }
}

impl<DB, INSP> InspectEvm for AlpenEvm<DB, INSP>
where
    DB: Database,
    INSP: Inspector<EthEvmContext<DB>>,
{
    type Inspector = INSP;

    fn set_inspector(&mut self, inspector: Self::Inspector) {
        self.inner.set_inspector(inspector);
    }

    fn inspect_one_tx(&mut self, tx: Self::Tx) -> Result<Self::ExecutionResult, Self::Error> {
        self.inner.ctx.set_tx(tx);
        AlpenRevmHandler::default().inspect_run(self)
    }
}

impl<DB, INSP> InspectCommitEvm for AlpenEvm<DB, INSP>
where
    DB: Database + DatabaseCommit,
    INSP: Inspector<EthEvmContext<DB>>,
{
}
