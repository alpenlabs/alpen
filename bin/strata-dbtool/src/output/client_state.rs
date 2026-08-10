//! Client state formatting implementations

use strata_csm_types::ClientState;
use strata_primitives::prelude::L1BlockCommitment;

use super::{helpers::porcelain_field, traits::Formattable};
use crate::output::helpers::porcelain_optional;

/// Client state update information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct ClientStateUpdateInfo {
    pub(crate) block: L1BlockCommitment,
    pub(crate) state: ClientState,
}

impl Formattable for ClientStateUpdateInfo {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field(
            "client_state_update.block",
            format!("{:?}", self.block),
        ));

        output.push(porcelain_field(
            "client_state_update.client_state.last_finalized_checkpoint",
            porcelain_optional(&self.state.get_last_finalized_checkpoint()),
        ));

        output.push(porcelain_field(
            "client_state_update.client_state.last_seen_checkpoint",
            porcelain_optional(&self.state.get_last_checkpoint()),
        ));

        output.join("\n")
    }
}
