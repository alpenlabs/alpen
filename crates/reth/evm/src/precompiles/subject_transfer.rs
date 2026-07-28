use alpen_reth_primitives::{
    account_id_to_bytes32, subject_id_to_bytes32, SubjectTransferCalldata,
    SubjectTransferIntentEvent,
};
use reth_evm::precompiles::PrecompileInput;
use revm::precompile::{PrecompileError, PrecompileOutput, PrecompileResult};
use revm_primitives::{Bytes, Log, LogData, U256};

use crate::{
    constants::SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
    utils::{address_to_subject, wei_to_sats},
};

/// Fixed raw EVM gas charged for subject-transfer precompile execution.
const SUBJECT_TRANSFER_BASE_GAS: u64 = 10_000;

/// Raw EVM gas charged per calldata byte handled by the subject-transfer precompile.
const SUBJECT_TRANSFER_CALLDATA_BYTE_GAS: u64 = 16;

/// Machine-readable failure reasons returned by the subject-transfer precompile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubjectTransferError {
    /// `IncorrectCallType()` — precompile was not reached via a direct CALL.
    IncorrectCallType,
    /// `MalformedCalldata()` — calldata too short to hold destination account and subject.
    MalformedCalldata,
    /// `NonIntegerAmount()` — value is not a whole number of satoshis.
    NonIntegerAmount,
    /// `IncorrectAmount()` — value is zero.
    IncorrectAmount,
    /// `OversizeAmount()` — value exceeds `u64::MAX` satoshis.
    OversizeAmount,
}

impl SubjectTransferError {
    /// The Solidity custom-error selector, `bytes4(keccak256(signature))`.
    const fn selector(self) -> [u8; 4] {
        match self {
            Self::IncorrectCallType => [0x7a, 0x5e, 0x63, 0xdc],
            Self::MalformedCalldata => [0x59, 0x17, 0x0b, 0xf0],
            Self::NonIntegerAmount => [0xf7, 0x73, 0x8c, 0x57],
            Self::IncorrectAmount => [0x69, 0x64, 0x0e, 0x72],
            Self::OversizeAmount => [0xaf, 0x3a, 0xf8, 0x70],
        }
    }

    /// ABI-encodes the error selector.
    fn abi_encode(self) -> Bytes {
        Bytes::copy_from_slice(&self.selector())
    }
}

/// Builds a gas-refunding revert carrying an ABI-encoded custom error.
fn revert_with_error(gas_used: u64, error: SubjectTransferError) -> PrecompileResult {
    Ok(PrecompileOutput::new_reverted(gas_used, error.abi_encode()))
}

/// Custom precompile to burn rollup native token and emit an inter-EE subject-transfer intent.
///
/// Calldata format: `[32 bytes: destination account][32 bytes: destination subject][data...]`.
pub(crate) fn subject_transfer_context_call(mut input: PrecompileInput<'_>) -> PrecompileResult {
    let gas_cost = subject_transfer_gas_cost(input.data.len())?;
    if gas_cost > input.gas {
        return Err(PrecompileError::OutOfGas);
    }

    if !input.is_direct_call() {
        return revert_with_error(gas_cost, SubjectTransferError::IncorrectCallType);
    }

    let Some(calldata) = SubjectTransferCalldata::decode(input.data) else {
        return revert_with_error(gas_cost, SubjectTransferError::MalformedCalldata);
    };

    let amount = match validate_transfer_amount(input.value) {
        Ok(amount) => amount,
        Err(error) => return revert_with_error(gas_cost, error),
    };

    let source_subject = address_to_subject(input.caller);
    let evt = SubjectTransferIntentEvent {
        amount,
        sourceSubject: subject_id_to_bytes32(&source_subject),
        destAccount: account_id_to_bytes32(&calldata.dest_account),
        destSubject: subject_id_to_bytes32(&calldata.dest_subject),
        transferData: Bytes::from(calldata.data),
    };

    input.internals.log(Log {
        address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
        data: LogData::from(&evt),
    });

    input
        .internals
        .set_balance(SUBJECT_TRANSFER_PRECOMPILE_ADDRESS, U256::ZERO)
        .map_err(|_| {
            PrecompileError::Fatal(
                "Failed to reset SUBJECT_TRANSFER_PRECOMPILE_ADDRESS account balance".into(),
            )
        })?;

    Ok(PrecompileOutput::new(gas_cost, Bytes::new()))
}

fn subject_transfer_gas_cost(calldata_len: usize) -> Result<u64, PrecompileError> {
    let calldata_len = u64::try_from(calldata_len).map_err(|_| {
        PrecompileError::Fatal("Subject transfer calldata length exceeds u64".into())
    })?;

    SUBJECT_TRANSFER_CALLDATA_BYTE_GAS
        .checked_mul(calldata_len)
        .and_then(|calldata_gas| SUBJECT_TRANSFER_BASE_GAS.checked_add(calldata_gas))
        .ok_or_else(|| PrecompileError::Fatal("Subject transfer gas cost overflow".into()))
}

fn validate_transfer_amount(amount_wei: U256) -> Result<u64, SubjectTransferError> {
    let (amount_sats, remainder_wei) = wei_to_sats(amount_wei);
    if !remainder_wei.is_zero() {
        return Err(SubjectTransferError::NonIntegerAmount);
    }

    let amount_sats: u64 = amount_sats
        .try_into()
        .map_err(|_| SubjectTransferError::OversizeAmount)?;

    if amount_sats == 0 {
        return Err(SubjectTransferError::IncorrectAmount);
    }

    Ok(amount_sats)
}

#[cfg(test)]
mod tests {
    use alloy_sol_types::SolEvent;
    use alpen_reth_primitives::SubjectTransferIntentEvent;
    use reth_evm::EvmInternals;
    use revm::{
        context::{BlockEnv, Journal, JournalEntry, JournalTr},
        database::EmptyDB,
        primitives::address,
    };
    use strata_acct_types::AccountId;
    use strata_identifiers::SubjectId;

    use super::*;
    use crate::utils::{u256_from, WEI_PER_BTC};

    const ONE_BTC_WEI: U256 = u256_from(WEI_PER_BTC);

    fn calldata() -> Vec<u8> {
        SubjectTransferCalldata {
            dest_account: AccountId::new([0x22; 32]),
            dest_subject: SubjectId::new([0x33; 32]),
            data: vec![0xaa, 0xbb],
        }
        .encode()
    }

    fn selector_of(bytes: &[u8]) -> [u8; 4] {
        bytes[..4].try_into().expect("selector bytes")
    }

    #[test]
    fn test_custom_error_selectors_match_signatures() {
        use revm_primitives::keccak256;

        let cases: [(SubjectTransferError, &str); 5] = [
            (
                SubjectTransferError::IncorrectCallType,
                "IncorrectCallType()",
            ),
            (
                SubjectTransferError::MalformedCalldata,
                "MalformedCalldata()",
            ),
            (SubjectTransferError::NonIntegerAmount, "NonIntegerAmount()"),
            (SubjectTransferError::IncorrectAmount, "IncorrectAmount()"),
            (SubjectTransferError::OversizeAmount, "OversizeAmount()"),
        ];

        for (err, sig) in cases {
            assert_eq!(
                err.selector(),
                keccak256(sig.as_bytes())[..4],
                "selector drift for {sig}"
            );
            assert_eq!(err.abi_encode().len(), 4);
        }
    }

    #[test]
    fn subject_transfer_rejects_delegatecall_apparent_value() {
        let calldata = calldata();
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller: address!("1111111111111111111111111111111111111111"),
            value: ONE_BTC_WEI,
            target_address: address!("2222222222222222222222222222222222222222"),
            bytecode_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        let output = subject_transfer_context_call(input).unwrap();

        assert!(output.reverted);
        assert_eq!(
            selector_of(&output.bytes),
            SubjectTransferError::IncorrectCallType.selector()
        );
    }

    #[test]
    fn subject_transfer_accepts_direct_call_value() {
        let calldata = calldata();
        let caller = address!("1111111111111111111111111111111111111111");
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller,
            value: ONE_BTC_WEI,
            target_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            bytecode_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        let output = subject_transfer_context_call(input).unwrap();

        assert!(!output.reverted);
    }

    #[test]
    fn subject_transfer_emits_intent_log() {
        let calldata = calldata();
        let caller = address!("1111111111111111111111111111111111111111");
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller,
            value: ONE_BTC_WEI,
            target_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            bytecode_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        subject_transfer_context_call(input).unwrap();

        let log = journal.logs.last().expect("subject-transfer log");
        assert_eq!(log.address, SUBJECT_TRANSFER_PRECOMPILE_ADDRESS);

        let event = SubjectTransferIntentEvent::decode_log(log).expect("decode event");
        assert_eq!(event.amount, 100_000_000);
        assert_eq!(
            event.sourceSubject,
            subject_id_to_bytes32(&address_to_subject(caller))
        );
        assert_eq!(
            event.destAccount,
            account_id_to_bytes32(&AccountId::new([0x22; 32]))
        );
        assert_eq!(
            event.destSubject,
            subject_id_to_bytes32(&SubjectId::new([0x33; 32]))
        );
        assert_eq!(event.transferData, Bytes::from_static(&[0xaa, 0xbb]));
    }

    #[test]
    fn subject_transfer_rejects_zero_value() {
        let calldata = calldata();
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller: address!("1111111111111111111111111111111111111111"),
            value: U256::ZERO,
            target_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            bytecode_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        let output = subject_transfer_context_call(input).unwrap();

        assert!(output.reverted);
        assert_eq!(
            selector_of(&output.bytes),
            SubjectTransferError::IncorrectAmount.selector()
        );
    }

    #[test]
    fn subject_transfer_rejects_non_integer_sats() {
        let calldata = calldata();
        let mut journal: Journal<EmptyDB, JournalEntry> = Journal::new(EmptyDB::new());
        let block_env = BlockEnv::default();
        let input = PrecompileInput {
            data: &calldata,
            gas: u64::MAX,
            caller: address!("1111111111111111111111111111111111111111"),
            value: ONE_BTC_WEI + U256::from(1),
            target_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            bytecode_address: SUBJECT_TRANSFER_PRECOMPILE_ADDRESS,
            internals: EvmInternals::new(&mut journal, &block_env),
        };

        let output = subject_transfer_context_call(input).unwrap();

        assert!(output.reverted);
        assert_eq!(
            selector_of(&output.bytes),
            SubjectTransferError::NonIntegerAmount.selector()
        );
    }
}
