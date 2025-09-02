//! Client state formatting implementations

use strata_primitives::prelude::L1BlockCommitment;
use strata_state::{client_state::ClientState, operation::SyncAction};

use super::{helpers::porcelain_field, traits::Formattable};
use crate::output::helpers::porcelain_optional;

/// Client state update information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct ClientStateUpdateInfo<'a> {
    pub block: L1BlockCommitment,
    pub state: ClientState,
    pub sync_actions: &'a Vec<SyncAction>,
}

impl<'a> Formattable for ClientStateUpdateInfo<'a> {
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

        // Format sync actions
        for sync_action in self.sync_actions {
            match sync_action {
                SyncAction::FinalizeEpoch(epoch) => {
                    output.push(porcelain_field(
                        "client_state_update.sync_action",
                        "FinalizeEpoch",
                    ));
                    output.push(porcelain_field(
                        "client_state_update.sync_action.epoch",
                        epoch.epoch(),
                    ));
                    output.push(porcelain_field(
                        "client_state_update.sync_action.last_slot",
                        epoch.last_slot(),
                    ));
                    output.push(porcelain_field(
                        "client_state_update.sync_action.last_blkid",
                        format!("{:?}", epoch.last_blkid()),
                    ));
                }
                SyncAction::UpdateCheckpointInclusion { .. } => {
                    output.push(porcelain_field(
                        "client_state_update.sync_action",
                        "UpdateCheckpointInclusion",
                    ));
                }
            }
        }

        output.join("\n")
    }
}
