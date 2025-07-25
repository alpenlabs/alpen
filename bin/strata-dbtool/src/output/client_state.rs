//! Client state formatting implementations

use strata_primitives::prelude::L1BlockCommitment;
use strata_state::{client_state::InternalState, l1::L1BlockId, operation::SyncAction};

use super::{helpers::porcelain_field, traits::Formattable};

/// Client state update information displayed to the user
#[derive(serde::Serialize)]
pub(crate) struct ClientStateUpdateInfo<'a> {
    pub update_index: u64,
    pub is_chain_active: bool,
    pub horizon_l1_height: u64,
    pub genesis_l1_height: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_l1_block: Option<&'a L1BlockId>,
    pub next_expected_l1_height: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tip_l1_block: Option<L1BlockCommitment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deepest_l1_block: Option<L1BlockCommitment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_internal_state: Option<&'a InternalState>,
    pub sync_actions: &'a Vec<SyncAction>,
}

impl<'a> Formattable for ClientStateUpdateInfo<'a> {
    fn format_porcelain(&self) -> String {
        let mut output = Vec::new();

        output.push(porcelain_field(
            "client_state_update.update_index",
            self.update_index,
        ));
        output.push(porcelain_field(
            "client_state_update.client_state.is_chain_active",
            if self.is_chain_active {
                "true"
            } else {
                "false"
            },
        ));
        output.push(porcelain_field(
            "client_state_update.client_state.horizon_l1_height",
            self.horizon_l1_height,
        ));
        output.push(porcelain_field(
            "client_state_update.client_state.genesis_l1_height",
            self.genesis_l1_height,
        ));

        if let Some(l1_block) = self.latest_l1_block {
            output.push(porcelain_field(
                "client_state_update.client_state.latest_l1_block",
                format!("{l1_block:?}"),
            ));
        }

        output.push(porcelain_field(
            "client_state_update.client_state.next_expected_l1_height",
            self.next_expected_l1_height,
        ));

        if let Some(tip_l1_block) = &self.tip_l1_block {
            output.push(porcelain_field(
                "client_state_update.client_state.tip_l1_block.height",
                tip_l1_block.height(),
            ));
            output.push(porcelain_field(
                "client_state_update.client_state.tip_l1_block.blkid",
                format!("{:?}", tip_l1_block.blkid()),
            ));
        }

        if let Some(deepest_l1_block) = &self.deepest_l1_block {
            output.push(porcelain_field(
                "client_state_update.client_state.deepest_l1_block.height",
                deepest_l1_block.height(),
            ));
            output.push(porcelain_field(
                "client_state_update.client_state.deepest_l1_block.blkid",
                format!("{:?}", deepest_l1_block.blkid()),
            ));
        }

        if let Some(last_internal_state) = self.last_internal_state {
            output.push(porcelain_field(
                "client_state_update.client_state.last_internal_state.blkid",
                format!("{:?}", last_internal_state.blkid()),
            ));
        }

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
                SyncAction::L2Genesis(block_id) => {
                    output.push(porcelain_field(
                        "client_state_update.sync_action",
                        "L2Genesis",
                    ));
                    output.push(porcelain_field(
                        "client_state_update.sync_action.blkid",
                        format!("{block_id:?}"),
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
