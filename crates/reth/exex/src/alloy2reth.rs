use alloy_eips::eip4895::Withdrawal as RethWithdrawal;
use alloy_rpc_types::Withdrawal as AlloyWithdrawal;
use rsp_primitives::genesis::Genesis;

/// A trait to convert from Alloy types to Reth types.
pub trait IntoReth<T> {
    fn into_reth(self) -> T;
}

impl IntoReth<RethWithdrawal> for AlloyWithdrawal {
    fn into_reth(self) -> RethWithdrawal {
        RethWithdrawal {
            index: self.index,
            validator_index: self.validator_index,
            amount: self.amount,
            address: self.address,
        }
    }
}

/// A trait to convert from Alloy genesis to RSP genesis.
pub trait IntoRspChainConfig {
    fn into_rsp(self) -> Genesis;
}

impl IntoRspChainConfig for alloy_genesis::ChainConfig {
    fn into_rsp(self) -> Genesis {
        Genesis::Custom(self)
    }
}
