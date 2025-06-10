use revm::{
    context::{ContextSetters, ContextTr, Evm as EvmCtx},
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EvmTr, PrecompileProvider,
    },
    inspector::{InspectorEvmTr, JournalExt},
    interpreter::{interpreter::EthInterpreter, Interpreter, InterpreterAction, InterpreterTypes},
    Inspector,
};

use crate::AlpenEvmPrecompiles;

#[allow(missing_debug_implementations)]
pub struct AlpenEvmInner<
    CTX,
    INSP,
    I = EthInstructions<EthInterpreter, CTX>,
    P = AlpenEvmPrecompiles,
> {
    pub evm_ctx: EvmCtx<CTX, INSP, I, P>,
}

impl<CTX, INSP, I, P> AlpenEvmInner<CTX, INSP, I, P>
where
    CTX: ContextTr,
    INSP: Inspector<CTX, I::InterpreterTypes>,
    I: InstructionProvider<
        Context = CTX,
        InterpreterTypes: InterpreterTypes<Output = InterpreterAction>,
    >,
    P: PrecompileProvider<CTX>,
{
    /// Creates a new instance of `AlpenEvmInner`.
    pub fn new(evm_ctx: EvmCtx<CTX, INSP, I, P>) -> Self {
        AlpenEvmInner { evm_ctx }
    }
}

impl<CTX, INSP, I, P> InspectorEvmTr for AlpenEvmInner<CTX, INSP, I, P>
where
    CTX: ContextTr<Journal: JournalExt> + ContextSetters,
    I: InstructionProvider<
        Context = CTX,
        InterpreterTypes: InterpreterTypes<Output = InterpreterAction>,
    >,
    INSP: Inspector<CTX, I::InterpreterTypes>,
    P: PrecompileProvider<CTX>,
{
    type Inspector = INSP;

    fn inspector(&mut self) -> &mut Self::Inspector {
        &mut self.evm_ctx.data.inspector
    }

    fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector) {
        (&mut self.evm_ctx.data.ctx, &mut self.evm_ctx.data.inspector)
    }

    fn run_inspect_interpreter(
        &mut self,
        interpreter: &mut Interpreter<
            <Self::Instructions as InstructionProvider>::InterpreterTypes,
        >,
    ) -> <<Self::Instructions as InstructionProvider>::InterpreterTypes as InterpreterTypes>::Output
    {
        self.evm_ctx.run_inspect_interpreter(interpreter)
    }
}

impl<CTX, INSP, I, P> EvmTr for AlpenEvmInner<CTX, INSP, I, P>
where
    CTX: ContextTr,
    I: InstructionProvider<
        Context = CTX,
        InterpreterTypes: InterpreterTypes<Output = InterpreterAction>,
    >,
    P: PrecompileProvider<CTX>,
{
    type Context = CTX;
    type Instructions = I;
    type Precompiles = P;

    fn run_interpreter(
        &mut self,
        interpreter: &mut Interpreter<
            <Self::Instructions as InstructionProvider>::InterpreterTypes,
        >,
    ) -> <<Self::Instructions as InstructionProvider>::InterpreterTypes as InterpreterTypes>::Output
    {
        let context = &mut self.evm_ctx.data.ctx;
        let instructions = &mut self.evm_ctx.instruction;
        interpreter.run_plain(instructions.instruction_table(), context)
    }

    fn ctx(&mut self) -> &mut Self::Context {
        &mut self.evm_ctx.data.ctx
    }

    fn ctx_ref(&self) -> &Self::Context {
        &self.evm_ctx.data.ctx
    }

    fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions) {
        (&mut self.evm_ctx.data.ctx, &mut self.evm_ctx.instruction)
    }

    fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles) {
        (&mut self.evm_ctx.data.ctx, &mut self.evm_ctx.precompiles)
    }
}
