use core::marker::PhantomData;

use revm::{
    context::{
        result::{EVMError, HaltReason, InvalidTransaction},
        Block, ContextTr, JournalTr, Transaction,
    },
    handler::{
        instructions::InstructionProvider, EvmTr, FrameResult, FrameTr, Handler, PrecompileProvider,
    },
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{interpreter::EthInterpreter, interpreter_action::FrameInit, InterpreterResult},
    state::EvmState,
    Database, Inspector,
};
use revm_primitives::U256;

use crate::{
    apis::validation,
    constants::DA_FEE_VAULT_ADDRESS,
    da_fee::{bounded_da_fee, calc_diff_size, DaStateAccess},
};

#[expect(
    missing_debug_implementations,
    reason = "Handler struct contains phantom data and doesn't need debug implementation"
)]
pub struct AlpenRevmHandler<EVM> {
    /// Per-block DA rate (wei per byte) used to charge the DA fee.
    da_rate: U256,
    pub _phantom: PhantomData<EVM>,
}

impl<EVM> AlpenRevmHandler<EVM> {
    /// Creates a handler that charges the DA fee at the given per-block rate.
    pub fn new(da_rate: U256) -> Self {
        Self {
            da_rate,
            _phantom: PhantomData,
        }
    }
}

impl<EVM> Default for AlpenRevmHandler<EVM> {
    fn default() -> Self {
        Self {
            da_rate: U256::ZERO,
            _phantom: PhantomData,
        }
    }
}

impl<EVM> Handler for AlpenRevmHandler<EVM>
where
    EVM: EvmTr<
        Context: ContextTr<Journal: JournalTr<State = EvmState>> + DaStateAccess,
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
        let caller = tx.caller();
        let basefee = block.basefee() as u128;
        let effective_gas_price = tx.effective_gas_price(basefee);

        let gas = exec_result.gas();
        let gas_used = (gas.spent() - gas.refunded() as u64) as u128;
        // Value of the gas budget the caller authorized but did not consume — this was
        // just refunded to the caller, so a DA fee bounded by it is always covered.
        let remaining_gas = (gas.remaining() as u128) + (gas.refunded() as u128);
        let remaining_value = U256::from(remaining_gas) * U256::from(effective_gas_price);

        // Credit all gas fees to the beneficiary (base fee + priority fee).
        context
            .journal_mut()
            .load_account_mut(beneficiary)?
            .incr_balance(U256::from(effective_gas_price * gas_used));

        // Charge the DA fee, drawn from the unused authorized gas budget. Skip
        // system/zero-fee calls (effective_gas_price == 0) and no-op when no rate is set.
        if effective_gas_price != 0 && self.da_rate != U256::ZERO {
            let diff_size = calc_diff_size(context.evm_state());
            let da_fee = bounded_da_fee(self.da_rate, diff_size, remaining_value);
            if da_fee != U256::ZERO {
                // Debit the caller and credit the vault through the journal so both
                // accounts are loaded/journaled properly. Mutating the state map directly
                // would leave the vault account unloaded and panic bundle assembly. The
                // debit is covered because `da_fee` is bounded by the caller's just-
                // refunded gas value.
                context
                    .journal_mut()
                    .load_account_mut(caller)?
                    .decr_balance(da_fee);
                context
                    .journal_mut()
                    .load_account_mut(DA_FEE_VAULT_ADDRESS)?
                    .incr_balance(da_fee);
            }
        }

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
        Context: ContextTr<Journal: JournalTr<State = EvmState>> + DaStateAccess,
        Precompiles: PrecompileProvider<EVM::Context, Output = InterpreterResult>,
        Instructions: InstructionProvider<
            Context = EVM::Context,
            InterpreterTypes = EthInterpreter,
        >,
    >,
{
    type IT = EthInterpreter;
}
