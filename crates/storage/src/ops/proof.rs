//! Proof database operation interface.

use strata_db_types::{traits::ProofDatabase, DbResult};
use strata_primitives::proof::{ProofContext, ProofKey};
use zkaleido::ProofReceiptWithMetadata;

use crate::{exec::*, instrumentation::components};

pub trait ProofDatabaseOpsExt: ProofDatabase {
    fn get_proof_by_key(&self, proof_key: ProofKey) -> DbResult<Option<ProofReceiptWithMetadata>> {
        self.get_proof(&proof_key)
    }
}

impl<T: ProofDatabase + ?Sized> ProofDatabaseOpsExt for T {}

inst_ops_simple! {
    (<D: ProofDatabaseOpsExt> => ProofDbOps, component = components::STORAGE_PROOF) {
        put_proof(proof_key: ProofKey, proof: ProofReceiptWithMetadata) => ();
        get_proof_by_key(proof_key: ProofKey) => Option<ProofReceiptWithMetadata>;
        del_proof(proof_key: ProofKey) => bool;
        put_proof_deps(proof_context: ProofContext, deps: Vec<ProofContext>) => ();
        get_proof_deps(proof_context: ProofContext) => Option<Vec<ProofContext>>;
        del_proof_deps(proof_context: ProofContext) => bool;
    }
}
