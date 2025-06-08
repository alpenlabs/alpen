use revm::{
    context::{
        result::{EVMError, HaltReason, InvalidTransaction},
        ContextTr, JournalOutput, JournalTr,
    },
    handler::{
        instructions::InstructionProvider, EthFrame, EvmTr, FrameResult, Handler,
        PrecompileProvider,
    },
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, InterpreterResult},
    Database, Inspector,
};

#[allow(missing_debug_implementations)]
pub struct AlpenRevmHandler<EVM> {
    pub _phantom: core::marker::PhantomData<EVM>,
}

impl<EVM> Default for AlpenRevmHandler<EVM> {
    fn default() -> Self {
        Self {
            _phantom: core::marker::PhantomData,
        }
    }
}

impl<EVM> Handler for AlpenRevmHandler<EVM>
where
    EVM: EvmTr<
        Context: ContextTr<Journal: JournalTr<FinalOutput = JournalOutput>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>;
    type Frame = EthFrame<
        EVM,
        EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>,
        <EVM::Instructions as InstructionProvider>::InterpreterTypes,
    >;
    type HaltReason = HaltReason;

    fn reward_beneficiary(
        &self,
        _evm: &mut Self::Evm,
        _exec_result: &mut FrameResult,
    ) -> Result<(), Self::Error> {
        // Skip beneficiary reward
        Ok(())
    }
}

impl<EVM> InspectorHandler for AlpenRevmHandler<EVM>
where
    EVM: InspectorEvmTr<
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
        Context: ContextTr<Journal: JournalTr<FinalOutput = JournalOutput>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type IT = EthInterpreter;
}
