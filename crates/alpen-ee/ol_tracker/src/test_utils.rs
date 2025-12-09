use alpen_ee_common::{
    EeAccountStateAtEpoch, MockOLClient, MockStorage, OLBlockOrEpoch, OLClientError, OLEpochSummary,
};
use strata_acct_types::BitcoinAmount;
use strata_ee_acct_types::EeAccountState;
use strata_identifiers::{Buf32, EpochCommitment, OLBlockCommitment, OLBlockId};

pub(crate) fn make_epoch_commitment(epoch: u32, slot: u64, id: u8) -> EpochCommitment {
    let mut bytes = [0u8; 32];
    bytes[0] = id;
    EpochCommitment::new(epoch, slot, OLBlockId::from(Buf32::new(bytes)))
}

pub(crate) fn make_block_commitment(slot: u64, id: u8) -> OLBlockCommitment {
    let mut bytes = [0u8; 32];
    bytes[0] = id;
    OLBlockCommitment::new(slot, OLBlockId::from(Buf32::new(bytes)))
}

pub(crate) fn make_ee_state(last_exec_blkid: [u8; 32]) -> EeAccountState {
    EeAccountState::new(last_exec_blkid, BitcoinAmount::zero(), vec![], vec![])
}

pub(crate) fn make_state_at_epoch(
    epoch: u32,
    slot: u64,
    block_id: u8,
    state_id: u8,
) -> EeAccountStateAtEpoch {
    let epoch_commitment = make_epoch_commitment(epoch, slot, block_id);
    let mut state_bytes = [0u8; 32];
    state_bytes[0] = state_id;
    let state = make_ee_state(state_bytes);
    EeAccountStateAtEpoch::new(epoch_commitment, state)
}

/// Creates a chain of epochs with specified terminal block IDs.
///
/// Each epoch is identified by its terminal block ID byte. The epoch number starts at 0
/// and increments for each entry. Slots are calculated as epoch * 10.
///
/// # Example
/// ```
/// // Creates epochs 0, 1, 2 with terminal block IDs [100, 101, 102]
/// let chain = create_epochs(&[100, 101, 102]);
/// assert_eq!(chain[0].epoch_commitment().epoch(), 0);
/// assert_eq!(chain[1].epoch_commitment().epoch(), 1);
/// assert_eq!(chain[2].epoch_commitment().epoch(), 2);
/// ```
pub(crate) fn create_epochs(terminal_ids: &[u8]) -> Vec<EeAccountStateAtEpoch> {
    terminal_ids
        .iter()
        .enumerate()
        .map(|(epoch, &id)| {
            let epoch = epoch as u32;
            let slot = epoch as u64 * 10;
            make_state_at_epoch(epoch, slot, id, id)
        })
        .collect()
}

/// Sets up mock client to return epochs from a pre-built chain.
/// The chain's epochs are indexed by their epoch number.
pub(crate) fn setup_mock_client_with_chain(
    mock_client: &mut MockOLClient,
    chain: Vec<EeAccountStateAtEpoch>,
) {
    mock_client.expect_epoch_summary().returning(move |epoch| {
        let epoch_idx = epoch as usize;
        if epoch_idx >= chain.len() {
            return Err(OLClientError::network("epoch not found"));
        }
        let prev_epoch = if epoch > 0 { epoch - 1 } else { epoch };
        let current = &chain[epoch_idx];
        Ok(OLEpochSummary::new(
            *current.epoch_commitment(),
            make_epoch_commitment(prev_epoch, prev_epoch as u64 * 10, 0),
            vec![],
        ))
    });
}

/// Sets up mock storage to return epochs from a pre-built chain.
/// Handles both terminal block and epoch queries.
pub(crate) fn setup_mock_storage_with_chain(
    mock_storage: &mut MockStorage,
    chain: Vec<EeAccountStateAtEpoch>,
) {
    mock_storage
        .expect_ee_account_state()
        .returning(move |block_or_slot| match block_or_slot {
            OLBlockOrEpoch::TerminalBlock(block_id) => {
                let id_byte = block_id.as_ref()[0];
                for state in &chain {
                    if state.epoch_commitment().last_blkid().as_ref()[0] == id_byte {
                        return Ok(Some(state.clone()));
                    }
                }
                Ok(None)
            }
            OLBlockOrEpoch::Epoch(epoch) => {
                let state = &chain[epoch as usize];
                Ok(Some(state.clone()))
            }
        });
}
