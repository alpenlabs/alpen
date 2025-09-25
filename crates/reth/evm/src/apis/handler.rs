use revm::{
    context::{
        result::{EVMError, HaltReason, InvalidTransaction},
        Block, Cfg, ContextTr, JournalTr, Transaction,
    },
    handler::{
        instructions::InstructionProvider, EvmTr, FrameResult, FrameTr, Handler, PrecompileProvider,
    },
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, interpreter_action::FrameInit, InterpreterResult},
    state::EvmState,
    Database, Inspector,
};
use revm_primitives::{hardfork::SpecId, U256};

use crate::{apis::validation, constants::BASEFEE_ADDRESS};

#[expect(
    missing_debug_implementations,
    reason = "Handler struct contains phantom data and doesn't need debug implementation"
)]
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
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
        Frame: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
    >,
{
    type Evm = EVM;
    type Error = EVMError<<<EVM::Context as ContextTr>::Db as Database>::Error, InvalidTransaction>;
    type HaltReason = HaltReason;

    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        exec_result: &mut FrameResult,
    ) -> Result<(), Self::Error> {
        let context = evm.ctx();
        let block = context.block();
        let tx = context.tx();
        let beneficiary = block.beneficiary();
        let basefee = block.basefee() as u128;
        let effective_gas_price = tx.effective_gas_price(basefee);

        // Calculate total gas used
        let gas = exec_result.gas();
        let gas_used = (gas.spent() - gas.refunded() as u64) as u128;

        // Calculate base fee in ETH (wei)
        let base_fee_total = basefee * gas_used;

        // Calculate coinbase/beneficiary reward (EIP-1559: effective_gas_price - basefee)
        let coinbase_gas_price = if context.cfg().spec().into().is_enabled_in(SpecId::LONDON) {
            effective_gas_price.saturating_sub(basefee)
        } else {
            effective_gas_price
        };
        let coinbase_reward = coinbase_gas_price * gas_used;

        // Transfer base fee to BASEFEE_ADDRESS
        let basefee_account = context.journal_mut().load_account(BASEFEE_ADDRESS)?;
        basefee_account.data.mark_touch();
        basefee_account.data.info.balance = basefee_account
            .data
            .info
            .balance
            .saturating_add(U256::from(base_fee_total));

        // Transfer remaining reward to beneficiary
        let coinbase_account = context.journal_mut().load_account(beneficiary)?;
        coinbase_account.data.mark_touch();
        coinbase_account.data.info.balance = coinbase_account
            .data
            .info
            .balance
            .saturating_add(U256::from(coinbase_reward));

        Ok(())
    }

    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        // uses the validation module to validate the environment with disables the 4844 transaction
        validation::validate_env(evm.ctx())
    }
}

impl<EVM> InspectorHandler for AlpenRevmHandler<EVM>
where
    EVM: InspectorEvmTr<
        Inspector: Inspector<<<Self as Handler>::Evm as EvmTr>::Context, EthInterpreter>,
        Context: ContextTr<Journal: JournalTr<State = EvmState>>,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type IT = EthInterpreter;
}
