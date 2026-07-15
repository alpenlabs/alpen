use strata_asm_proto_checkpoint_types::{CheckpointTip, TerminalHeaderComplement};
use strata_identifiers::{Buf32, Epoch, OLBlockId};
use strata_ol_chain_types::{BlockFlags, OLBlockHeader};
use thiserror::Error;

/// Errors produced while reconstructing a checkpoint terminal header.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum TerminalHeaderReconstructionError {
    /// The reconstructed header does not match the checkpoint's terminal commitment.
    #[error(
        "terminal blkid mismatch at epoch {epoch} (expected {expected:?}, reconstructed {reconstructed:?})"
    )]
    BlockIdMismatch {
        /// Epoch committed by the checkpoint tip.
        epoch: Epoch,
        /// Block ID committed by the checkpoint tip.
        expected: OLBlockId,
        /// Block ID computed from the reconstructed header.
        reconstructed: OLBlockId,
    },
}

/// Reconstructs the unsigned terminal [`OLBlockHeader`] committed by a checkpoint.
///
/// The checkpoint tip supplies the terminal slot, epoch, and expected block ID. The complement
/// supplies fields that cannot be derived from state reconstruction, while `state_root` is the
/// root computed from the reconstructed post-epoch state.
pub fn reconstruct_terminal_header(
    tip: &CheckpointTip,
    complement: &TerminalHeaderComplement,
    state_root: Buf32,
) -> Result<OLBlockHeader, TerminalHeaderReconstructionError> {
    let terminal = tip.l2_commitment();
    let mut flags = BlockFlags::zero();
    flags.set_is_terminal(true);

    let header = OLBlockHeader::new(
        complement.timestamp(),
        flags,
        terminal.slot(),
        tip.epoch,
        *complement.parent_blkid(),
        *complement.body_root(),
        state_root,
        *complement.logs_root(),
    );
    let reconstructed = header.compute_blkid();
    let expected = *terminal.blkid();
    if reconstructed != expected {
        return Err(TerminalHeaderReconstructionError::BlockIdMismatch {
            epoch: tip.epoch,
            expected,
            reconstructed,
        });
    }

    Ok(header)
}

#[cfg(test)]
mod tests {
    use strata_asm_proto_checkpoint_types::{CheckpointTip, TerminalHeaderComplement};
    use strata_identifiers::{Buf32, OLBlockCommitment, OLBlockId};
    use strata_ol_chain_types::{BlockFlags, OLBlockHeader};

    use super::{reconstruct_terminal_header, TerminalHeaderReconstructionError};

    fn fixture() -> (
        CheckpointTip,
        TerminalHeaderComplement,
        Buf32,
        OLBlockHeader,
    ) {
        let epoch = 7;
        let slot = 42;
        let state_root = Buf32::from([4; 32]);
        let complement = TerminalHeaderComplement::new(
            1_700_000_000,
            OLBlockId::from(Buf32::from([1; 32])),
            Buf32::from([2; 32]),
            Buf32::from([3; 32]),
        );
        let mut flags = BlockFlags::zero();
        flags.set_is_terminal(true);
        let expected_header = OLBlockHeader::new(
            complement.timestamp(),
            flags,
            slot,
            epoch,
            *complement.parent_blkid(),
            *complement.body_root(),
            state_root,
            *complement.logs_root(),
        );
        let tip = CheckpointTip::new(
            epoch,
            100,
            OLBlockCommitment::new(slot, expected_header.compute_blkid()),
        );

        (tip, complement, state_root, expected_header)
    }

    #[test]
    fn reconstructs_and_validates_terminal_header() {
        let (tip, complement, state_root, expected_header) = fixture();

        let header = reconstruct_terminal_header(&tip, &complement, state_root)
            .expect("reconstruct terminal header");

        assert_eq!(header, expected_header);
    }

    #[test]
    fn rejects_mismatched_checkpoint_terminal() {
        let (tip, complement, state_root, _) = fixture();
        let mismatched_tip = CheckpointTip::new(
            tip.epoch,
            tip.l1_height(),
            OLBlockCommitment::new(
                tip.l2_commitment().slot(),
                OLBlockId::from(Buf32::from([9; 32])),
            ),
        );

        let error = reconstruct_terminal_header(&mismatched_tip, &complement, state_root)
            .expect_err("mismatched terminal must fail");

        assert!(matches!(
            error,
            TerminalHeaderReconstructionError::BlockIdMismatch {
                epoch: 7,
                expected,
                ..
            } if expected == OLBlockId::from(Buf32::from([9; 32]))
        ));
    }
}
