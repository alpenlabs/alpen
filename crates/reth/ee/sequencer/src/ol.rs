use alpen_ee_node::ol::OLClient;
use alpen_ee_primitives::{indexed_vec::IndexedVec, AccountAddress, OlBlockId};

use crate::{batch::EEUpdate, message::InboundMsg};

pub trait OLClientExt: OLClient {
    /// Get all inbound account messages that were added to OL state in given `ol_blockid`
    /// equivalent to diff of account message queue between given ol block and its parent
    fn new_account_messages(
        &self,
        ol_blockid: OlBlockId,
        account: AccountAddress,
    ) -> Option<IndexedVec<InboundMsg>>;

    /// Submit an EE state update to OL
    fn submit_ee_update(&self, update: EEUpdate) -> Result<(), EEUpdateError>;
}

#[derive(Debug, Clone)]
pub enum EEUpdateError {
    Other(String),
}
