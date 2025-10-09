use strata_csm_types::{ClientState, ClientUpdateOutput};
use strata_db::{DbResult, traits::*};
use strata_primitives::l1::L1BlockCommitment;

use super::schemas::ClientUpdateOutputSchema;
use crate::define_sled_database;

define_sled_database!(
    pub struct ClientStateDBSled {
        client_update_tree: ClientUpdateOutputSchema,
    }
);

impl ClientStateDatabase for ClientStateDBSled {
    fn put_client_update(
        &self,
        block: L1BlockCommitment,
        output: ClientUpdateOutput,
    ) -> DbResult<()> {
        Ok(self.client_update_tree.insert(&block, &output)?)
    }

    fn get_client_update(&self, block: L1BlockCommitment) -> DbResult<Option<ClientUpdateOutput>> {
        Ok(self.client_update_tree.get(&block)?)
    }

    fn get_latest_client_state(&self) -> DbResult<Option<(L1BlockCommitment, ClientState)>> {
        // Relying on the lexicographical order of L1BlockCommitment.
        let mut iter = self.client_update_tree.iter().rev();
        let res = iter.next().map(|r| r.map(|(k, v)| (k, v.into_state())));
        Ok(res.transpose()?)
    }
}

#[cfg(feature = "test_utils")]
#[cfg(test)]
mod tests {
    use strata_db_tests::client_state_db_tests;

    use super::*;
    use crate::sled_db_test_setup;

    sled_db_test_setup!(ClientStateDBSled, client_state_db_tests);
}
