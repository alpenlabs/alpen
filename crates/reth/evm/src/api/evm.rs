use revm::{
    context::{ContextError, ContextSetters, ContextTr, Evm as EvmCtx, FrameStack},
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EthFrame, EvmTr, FrameInitOrResult, FrameTr, ItemOrResult, PrecompileProvider,
    },
    inspector::{InspectorEvmTr, JournalExt},
    interpreter::{interpreter::EthInterpreter, InterpreterAction, InterpreterTypes},
    Database, Inspector,
};

use crate::AlpenEvmPrecompiles;

#[allow(missing_debug_implementations)]
pub struct AlpenEvmInner<
    CTX,
    INSP,
    I = EthInstructions<EthInterpreter, CTX>,
    P = AlpenEvmPrecompiles,
> {
    pub evm_ctx: EvmCtx<CTX, INSP, I, P, EthFrame>,
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
    pub fn new(evm_ctx: EvmCtx<CTX, INSP, I, P, EthFrame>) -> Self {
        AlpenEvmInner { evm_ctx }
    }
}

impl<CTX: ContextTr, INSP> InspectorEvmTr for AlpenEvmInner<CTX, INSP>
where
    CTX: ContextSetters<Journal: JournalExt>,
    INSP: Inspector<CTX, EthInterpreter>,
{
    type Inspector = INSP;

    fn inspector(&mut self) -> &mut Self::Inspector {
        &mut self.evm_ctx.inspector
    }

    fn ctx_inspector(&mut self) -> (&mut Self::Context, &mut Self::Inspector) {
        (&mut self.evm_ctx.ctx, &mut self.evm_ctx.inspector)
    }

    fn ctx_inspector_frame(
        &mut self,
    ) -> (&mut Self::Context, &mut Self::Inspector, &mut Self::Frame) {
        self.evm_ctx.ctx_inspector_frame()
    }

    fn ctx_inspector_frame_instructions(
        &mut self,
    ) -> (
        &mut Self::Context,
        &mut Self::Inspector,
        &mut Self::Frame,
        &mut Self::Instructions,
    ) {
        self.evm_ctx.ctx_inspector_frame_instructions()
    }
}

impl<CTX: ContextTr, INSP> EvmTr for AlpenEvmInner<CTX, INSP>
where
    CTX: ContextTr,
{
    type Context = CTX;
    type Instructions = EthInstructions<EthInterpreter, CTX>;
    type Precompiles = AlpenEvmPrecompiles;
    type Frame = EthFrame<EthInterpreter>;

    fn ctx(&mut self) -> &mut Self::Context {
        &mut self.evm_ctx.ctx
    }

    fn ctx_ref(&self) -> &Self::Context {
        self.evm_ctx.ctx_ref()
    }

    fn ctx_instructions(&mut self) -> (&mut Self::Context, &mut Self::Instructions) {
        self.evm_ctx.ctx_instructions()
    }

    fn ctx_precompiles(&mut self) -> (&mut Self::Context, &mut Self::Precompiles) {
        self.evm_ctx.ctx_precompiles()
    }

    fn frame_stack(&mut self) -> &mut FrameStack<Self::Frame> {
        self.evm_ctx.frame_stack()
    }

    fn frame_init(
        &mut self,
        frame_input: <Self::Frame as FrameTr>::FrameInit,
    ) -> Result<
        ItemOrResult<&mut Self::Frame, <Self::Frame as FrameTr>::FrameResult>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        self.evm_ctx.frame_init(frame_input)
    }

    fn frame_run(
        &mut self,
    ) -> Result<
        FrameInitOrResult<Self::Frame>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        self.evm_ctx.frame_run()
    }

    fn frame_return_result(
        &mut self,
        frame_result: <Self::Frame as FrameTr>::FrameResult,
    ) -> Result<
        Option<<Self::Frame as FrameTr>::FrameResult>,
        ContextError<<<Self::Context as ContextTr>::Db as Database>::Error>,
    > {
        self.evm_ctx.frame_return_result(frame_result)
    }
}
