use alloy_eips::eip4895::Withdrawal as RethWithdrawal;
use alloy_rpc_types::Withdrawal as AlloyWithdrawal;

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
pub trait IntoRspGenesis {
    fn try_into_rsp(self) -> Result<rsp_primitives::genesis::Genesis, serde_json::Error>;
}

impl IntoRspGenesis for alloy_genesis::Genesis {
    fn try_into_rsp(self) -> Result<rsp_primitives::genesis::Genesis, serde_json::Error> {
        let genesis_str = serde_json::to_string(&self)?;
        Ok(rsp_primitives::genesis::Genesis::Custom(genesis_str))
    }
}
