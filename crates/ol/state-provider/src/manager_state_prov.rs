//! State provider impl that wraps the OL state manager.

use std::sync::Arc;

use futures::TryFutureExt;
use strata_db_types::DbError;
use strata_identifiers::OLBlockCommitment;
use strata_ol_state_support_types::MemoryStateBaseLayer;
use strata_storage::OLStateManager;

use crate::state_provider::StateProvider;

/// [`StateProvider`] impl directly wrapping the [`OLStateManager`] handle.
#[expect(missing_debug_implementations, reason = "can't")]
pub struct OLStateManagerProviderImpl {
    manager: Arc<OLStateManager>,
}

impl OLStateManagerProviderImpl {
    pub fn new(manager: Arc<OLStateManager>) -> Self {
        Self { manager }
    }
}

impl StateProvider for OLStateManagerProviderImpl {
    type State = MemoryStateBaseLayer;
    type Error = DbError;

    fn get_state_for_tip_async(
        &self,
        tip: OLBlockCommitment,
    ) -> impl Future<Output = Result<Option<Self::State>, Self::Error>> + Send {
        self.manager
            .get_toplevel_ol_state_async(tip)
            .map_ok(|opt| opt.map(|state| MemoryStateBaseLayer::new(state.as_ref().clone())))
    }

    fn get_state_for_tip_blocking(
        &self,
        tip: OLBlockCommitment,
    ) -> Result<Option<Self::State>, Self::Error> {
        self.manager
            .get_toplevel_ol_state_blocking(tip)
            .map(|opt| opt.map(|state| MemoryStateBaseLayer::new(state.as_ref().clone())))
    }
}
