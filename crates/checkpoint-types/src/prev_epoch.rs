//! Where an epoch's L1 range starts: the last L1 block of the epoch before it.

use std::future::Future;

use strata_identifiers::{EpochCommitment, L1BlockCommitment};

use crate::EpochSummary;

/// Returns the last L1 block the epoch before `epoch` processed, which is
/// where `epoch`'s L1 range starts.
///
/// The epoch before is the one `epoch`'s own summary links to, not the
/// canonical summary for the previous epoch number, so the result follows
/// `epoch`'s chain. The genesis epoch has no epoch before it: it starts at
/// `genesis_l1_block`, the L1 anchor, and needs no lookup.
///
/// `get_summary` returns the stored summary of an epoch and reports a missing
/// one as an error of its own.
///
/// # Errors
///
/// Returns the first error `get_summary` returns.
///
/// # Panics
///
/// Panics if `get_summary` returns a genesis summary for an epoch after
/// genesis.
pub fn prev_epoch_last_l1<E>(
    epoch: EpochCommitment,
    genesis_l1_block: L1BlockCommitment,
    mut get_summary: impl FnMut(EpochCommitment) -> Result<EpochSummary, E>,
) -> Result<L1BlockCommitment, E> {
    if epoch.epoch() == 0 {
        return Ok(genesis_l1_block);
    }
    let prev = linked_prev_epoch(get_summary(epoch)?);
    Ok(*get_summary(prev)?.new_l1())
}

/// [`prev_epoch_last_l1`] for callers whose summary lookup is async.
///
/// # Errors
///
/// Returns the first error `get_summary` returns.
///
/// # Panics
///
/// Panics if `get_summary` returns a genesis summary for an epoch after
/// genesis.
pub async fn prev_epoch_last_l1_async<E, F, Fut>(
    epoch: EpochCommitment,
    genesis_l1_block: L1BlockCommitment,
    mut get_summary: F,
) -> Result<L1BlockCommitment, E>
where
    F: FnMut(EpochCommitment) -> Fut,
    Fut: Future<Output = Result<EpochSummary, E>>,
{
    if epoch.epoch() == 0 {
        return Ok(genesis_l1_block);
    }
    let prev = linked_prev_epoch(get_summary(epoch).await?);
    Ok(*get_summary(prev).await?.new_l1())
}

/// Returns the epoch that `summary`, the summary of an epoch after genesis,
/// links to.
fn linked_prev_epoch(summary: EpochSummary) -> EpochCommitment {
    summary
        .get_prev_epoch_commitment()
        .expect("a summary of an epoch after genesis links to a previous epoch")
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, future::Ready};

    use futures::executor::block_on;
    use strata_identifiers::{Buf32, L1BlockId, OLBlockCommitment, OLBlockId};

    use super::*;

    fn block(slot: u64, byte: u8) -> OLBlockCommitment {
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([byte; 32])))
    }

    fn l1_block(height: u32, byte: u8) -> L1BlockCommitment {
        L1BlockCommitment::new(height, L1BlockId::from(Buf32::from([byte; 32])))
    }

    /// Two summaries for epoch 1 on different forks, and the summary of
    /// epoch 2 on the fork whose epoch 1 ends on L1 height 5.
    struct Fork {
        summaries: HashMap<EpochCommitment, EpochSummary>,
        epoch_2: EpochCommitment,
        linked_last_l1: L1BlockCommitment,
    }

    fn fork() -> Fork {
        let genesis = block(0, 0);
        let linked = block(10, 1);
        let other = block(11, 2);
        let linked_last_l1 = l1_block(5, 1);
        let summaries = [
            EpochSummary::new(1, linked, genesis, linked_last_l1, Buf32::zero()),
            EpochSummary::new(1, other, genesis, l1_block(7, 2), Buf32::zero()),
            EpochSummary::new(2, block(20, 3), linked, l1_block(9, 3), Buf32::zero()),
        ];
        let epoch_2 = summaries[2].get_epoch_commitment();
        Fork {
            summaries: summaries
                .into_iter()
                .map(|summary| (summary.get_epoch_commitment(), summary))
                .collect(),
            epoch_2,
            linked_last_l1,
        }
    }

    #[test]
    fn test_genesis_epoch_starts_at_l1_anchor_without_lookup() {
        let genesis = EpochCommitment::from_terminal(0, block(0, 0));
        let anchor = l1_block(3, 9);

        let start = prev_epoch_last_l1(genesis, anchor, |epoch| -> Result<EpochSummary, ()> {
            panic!("looked up {epoch} for the genesis epoch")
        });

        assert_eq!(start, Ok(anchor));
    }

    #[test]
    fn test_follows_the_epochs_own_link_on_a_fork() {
        let fork = fork();

        let start = prev_epoch_last_l1(fork.epoch_2, l1_block(0, 0), |epoch| {
            fork.summaries.get(&epoch).copied().ok_or(epoch)
        });

        assert_eq!(start, Ok(fork.linked_last_l1));
    }

    #[test]
    fn test_missing_summary_returns_the_lookup_error() {
        let fork = fork();
        let missing = EpochCommitment::from_terminal(3, block(30, 4));

        let start = prev_epoch_last_l1(missing, l1_block(0, 0), |epoch| {
            fork.summaries.get(&epoch).copied().ok_or(epoch)
        });

        assert_eq!(start, Err(missing));
    }

    #[test]
    fn test_missing_linked_summary_returns_the_lookup_error() {
        let mut fork = fork();
        let linked = fork.summaries[&fork.epoch_2]
            .get_prev_epoch_commitment()
            .expect("epoch 2 links to epoch 1");
        fork.summaries.remove(&linked);

        let start = prev_epoch_last_l1(fork.epoch_2, l1_block(0, 0), |epoch| {
            fork.summaries.get(&epoch).copied().ok_or(epoch)
        });

        assert_eq!(start, Err(linked));
    }

    #[test]
    fn test_async_genesis_epoch_starts_at_l1_anchor_without_lookup() {
        let genesis = EpochCommitment::from_terminal(0, block(0, 0));
        let anchor = l1_block(3, 9);

        let start = block_on(prev_epoch_last_l1_async(
            genesis,
            anchor,
            |epoch| -> Ready<Result<EpochSummary, ()>> {
                panic!("looked up {epoch} for the genesis epoch")
            },
        ));

        assert_eq!(start, Ok(anchor));
    }

    #[test]
    fn test_async_lookup_follows_the_epochs_own_link_on_a_fork() {
        let fork = fork();

        let start = block_on(prev_epoch_last_l1_async(
            fork.epoch_2,
            l1_block(0, 0),
            |epoch| {
                let summary = fork.summaries.get(&epoch).copied().ok_or(epoch);
                async move { summary }
            },
        ));

        assert_eq!(start, Ok(fork.linked_last_l1));
    }
}
