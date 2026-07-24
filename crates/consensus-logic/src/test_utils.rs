use strata_identifiers::{
    Buf32, Epoch, L1BlockCommitment, L1BlockId, OLBlockCommitment, OLBlockId,
};
use strata_primitives::EpochCommitment;

pub(crate) fn make_buf32(byte: u8) -> Buf32 {
    Buf32([byte; 32])
}

pub(crate) fn make_ol_block_id(byte: u8) -> OLBlockId {
    OLBlockId::from(make_buf32(byte))
}

pub(crate) fn make_l1_block_id(byte: u8) -> L1BlockId {
    L1BlockId::from(make_buf32(byte))
}

pub(crate) fn make_ol_block_commitment(slot: u64, id_byte: u8) -> OLBlockCommitment {
    OLBlockCommitment::new(slot, make_ol_block_id(id_byte))
}

pub(crate) fn make_l1_block_commitment(height: u32, id_byte: u8) -> L1BlockCommitment {
    L1BlockCommitment::new(height, make_l1_block_id(id_byte))
}

pub(crate) fn make_epoch_commitment(epoch: Epoch, slot: u64, id_byte: u8) -> EpochCommitment {
    EpochCommitment::new(epoch, slot, make_ol_block_id(id_byte))
}
