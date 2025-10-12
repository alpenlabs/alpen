use std::ops::AddAssign;

use bitcoin::Work;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtcWork(Work);

impl Default for BtcWork {
    fn default() -> Self {
        Self(Work::from_le_bytes([0u8; 32]))
    }
}

impl From<Work> for BtcWork {
    fn from(work: Work) -> Self {
        Self(work)
    }
}

impl From<BtcWork> for Work {
    fn from(btc_work: BtcWork) -> Self {
        btc_work.0
    }
}

impl AddAssign for BtcWork {
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0 + rhs.0;
    }
}

impl BorshSerialize for BtcWork {
    fn serialize<W: borsh::io::Write>(&self, writer: &mut W) -> borsh::io::Result<()> {
        BorshSerialize::serialize(&self.0.to_le_bytes(), writer)
    }
}

impl BorshDeserialize for BtcWork {
    fn deserialize_reader<R: borsh::io::Read>(reader: &mut R) -> borsh::io::Result<Self> {
        let bytes = <[u8; 32]>::deserialize_reader(reader)?;
        Ok(Self(Work::from_le_bytes(bytes)))
    }
}

impl<'a> arbitrary::Arbitrary<'a> for BtcWork {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let bytes = <[u8; 32]>::arbitrary(u)?;
        Ok(Self(Work::from_le_bytes(bytes)))
    }
}
