use strata_acct_types::{AccumulatorClaim, RawMerkleProof};
use strata_identifiers::{AccountId, OLTxId, Slot};

use super::{object::IChainObj, snark_account_update::ISauTransaction};
use crate::{ProofSatisfier, transaction::TxTypeId};

/// Generic transaction interface type.
pub trait ITransaction: IChainObj {
    // Common tx components.
    type Constraints: ITxConstraints;
    type Proofs: ITxProofs;

    // Transaction subtypes.
    type Gam: IGamTransaction;
    type Sau: ISauTransaction;

    /// Computes the txid of the transaction.
    fn compute_txid(&self) -> OLTxId;

    /// Gets the transaction subtype.
    fn tydata(&self) -> TxTyData<Self>;
}

pub enum TxTyData<T: ITransaction> {
    /// Generic account message transactions.
    GenericAcctMessage(T::Gam),

    /// Snark account update transactions.
    SnarkAcctUpdate(T::Sau),
}

impl<T: ITransaction> TxTyData<T> {
    pub fn target(&self) -> Option<AccountId> {
        match self {
            TxTyData::GenericAcctMessage(gam) => Some(gam.target()),
            TxTyData::SnarkAcctUpdate(sau) => Some(sau.target()),
        }
    }

    /// Gets the low-level type ID of the transaction.
    pub fn type_id(&self) -> TxTypeId {
        match self {
            Self::GenericAcctMessage(_) => TxTypeId::GenericAccountMessage,
            Self::SnarkAcctUpdate(_) => TxTypeId::SnarkAccountUpdate,
        }
    }

    pub fn as_generic_acct_msg(&self) -> Option<&T::Gam> {
        match self {
            Self::GenericAcctMessage(gam) => Some(gam),
            _ => None,
        }
    }

    pub fn as_snark_acct_update(&self) -> Option<&T::Sau> {
        match self {
            Self::SnarkAcctUpdate(sau) => Some(sau),
            _ => None,
        }
    }
}

/// Generic generic account message transaction.
///
/// Right now this can only be used by the sequencer in testing scenarios.
pub trait IGamTransaction: IChainObj + ITargetTx {
    // TODO
}

/// Tx constraints.
pub trait ITxConstraints {
    fn min_slot(&self) -> Option<Slot>;
    fn max_slot(&self) -> Option<Slot>;
}

/// Abstraction over transaction proof container.
pub trait ITxProofs {
    fn num_predicate_satisfiers(&self) -> usize;
    fn get_predicate_satisfier(&self, idx: usize) -> Option<ProofSatisfier>;
    fn num_accumulator_proofs(&self) -> usize;
    fn get_accumulator_proof(&self, idx: usize) -> Option<RawMerkleProof>;
}

/// Describes transactions that have a target.
pub trait ITargetTx {
    /// Gets the "target" of the operation.
    fn target(&self) -> AccountId;
}

/// Describes refs a tx makes to accumulators on the ledger.
pub trait ILedgerRefs {
    fn num_l1_block_ref_claims(&self) -> usize;
    fn get_l1_block_ref_claim(&self, idx: usize) -> Option<AccumulatorClaim>;
}
