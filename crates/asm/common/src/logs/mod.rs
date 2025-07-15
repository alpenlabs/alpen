use std::{any::Any, fmt::Debug};

mod deposit;
use borsh::{BorshDeserialize, BorshSerialize};
use strata_msg_fmt::{Msg, OwnedMsg, TypeId};

use crate::{AsmError, Mismatched};

pub(crate) trait AsmLog: Debug + BorshSerialize + BorshDeserialize {
    fn as_dyn_any(&self) -> &dyn Any;

    fn ty() -> TypeId;

    fn to_owned_msg(&self) -> Result<OwnedMsg, AsmError> {
        let ty = Self::ty();
        let body = borsh::to_vec(&self).map_err(|e| AsmError::TypeIdSerialization(ty, e))?;
        Ok(OwnedMsg::new(ty, body)?)
    }

    fn from_msg(msg: impl Msg) -> Result<Self, AsmError> {
        if msg.ty() != Self::ty() {
            return Err(AsmError::TypeIdMismatch(Mismatched {
                expected: Self::ty(),
                actual: msg.ty(),
            }));
        }

        borsh::from_slice(msg.body()).map_err(|e| AsmError::TypeIdDeserialization(Self::ty(), e))
    }
}
