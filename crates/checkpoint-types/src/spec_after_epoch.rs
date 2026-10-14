//! The spec of the epoch after an epoch, selected from the data a node stores.
//!
//! Every OL driver selects the next epoch's spec here: the chain worker,
//! checkpoint promotion and the sequencer's startup checks, block assembly,
//! the checkpoint builder, the mempool and the status RPC. They differ only in
//! how they read summaries and manifests. The rule itself is
//! [`select_next_epoch_spec`]; this module finds the L1 range it reads.

use std::future::Future;

use strata_identifiers::{EpochCommitment, L1BlockCommitment};
use strata_ol_chain_types_v1::AsmManifest;
use strata_ol_state_types::{OLSpecId, OLSpecVersions};
use strata_ol_stf::{
    select_next_epoch_spec, EpochL1Range, EpochSpecSelectionError, InvalidEpochL1Range,
};
use thiserror::Error;

use crate::EpochSummary;

/// Errors returned by [`select_spec_after_epoch`] and
/// [`select_spec_after_epoch_async`].
#[derive(Debug, Error)]
pub enum SelectSpecAfterEpochError<E> {
    /// Looking up a stored epoch summary failed.
    #[error("look up the epoch summary of {epoch}: {source}")]
    SummaryLookup {
        epoch: EpochCommitment,
        #[source]
        source: E,
    },

    /// No summary is stored for the epoch or the epoch before it.
    #[error("missing the epoch summary of {0}")]
    MissingEpochSummary(EpochCommitment),

    /// The epoch's last L1 block does not follow the previous epoch's.
    #[error("epoch L1 range: {0}")]
    InvalidL1Range(#[from] InvalidEpochL1Range),

    /// Selecting the spec from the epoch's last manifest failed, or the next
    /// epoch runs a spec this binary does not implement.
    #[error(transparent)]
    Selection(#[from] EpochSpecSelectionError<E>),
}

/// Selects the spec of the epoch after `terminal`, the last block of an epoch,
/// with [`select_next_epoch_spec`].
///
/// `spec_versions` and `epoch_last_l1_block` come from `terminal`'s state:
/// its spec versions and the last L1 block it processed. The epoch's L1 range
/// starts at the last L1 block of the epoch before it. That is the epoch
/// `terminal`'s own summary links to, not the canonical summary for the
/// previous epoch number, so the range follows `terminal`'s chain. The genesis
/// epoch has no epoch before it: it starts at `genesis_l1_block`, the L1
/// anchor, and needs no summary.
///
/// `get_summary` returns the stored summary of an epoch and `get_manifest` the
/// stored manifest of an L1 block, each `None` if none is stored.
/// `get_manifest` must look the manifest up by block, not by height. Then it
/// returns the manifest of the epoch's own last L1 block even after a reorg
/// replaced that block, and never another block's manifest at the same height.
/// It is called only if the epoch processed an L1 block.
///
/// # Errors
///
/// Returns an error if a lookup fails, a summary or the manifest is missing,
/// the epoch's last L1 block does not follow the previous epoch's, or
/// [`select_next_epoch_spec`] fails. That includes
/// [`EpochSpecSelectionError::UpgradeRequired`] when the next epoch runs a spec
/// this binary does not implement.
///
/// # Panics
///
/// Panics if `get_summary` returns a genesis summary for an epoch after
/// genesis.
pub fn select_spec_after_epoch<E>(
    terminal: EpochCommitment,
    spec_versions: OLSpecVersions,
    epoch_last_l1_block: L1BlockCommitment,
    genesis_l1_block: L1BlockCommitment,
    get_summary: impl FnMut(EpochCommitment) -> Result<Option<EpochSummary>, E>,
    get_manifest: impl FnOnce(L1BlockCommitment) -> Result<Option<AsmManifest>, E>,
) -> Result<OLSpecId, SelectSpecAfterEpochError<E>> {
    let prev_epoch_last_l1_block =
        resolve_prev_epoch_last_l1_block(terminal, genesis_l1_block, get_summary)?;
    let range = EpochL1Range::new(prev_epoch_last_l1_block, epoch_last_l1_block)?;
    Ok(select_next_epoch_spec(spec_versions, range, |block| {
        get_manifest(*block)
    })?)
}

/// [`select_spec_after_epoch`] for callers whose lookups are async.
///
/// # Errors
///
/// Returns the errors [`select_spec_after_epoch`] returns.
///
/// # Panics
///
/// Panics if `get_summary` returns a genesis summary for an epoch after
/// genesis.
pub async fn select_spec_after_epoch_async<E, S, SFut, M, MFut>(
    terminal: EpochCommitment,
    spec_versions: OLSpecVersions,
    epoch_last_l1_block: L1BlockCommitment,
    genesis_l1_block: L1BlockCommitment,
    get_summary: S,
    get_manifest: M,
) -> Result<OLSpecId, SelectSpecAfterEpochError<E>>
where
    S: FnMut(EpochCommitment) -> SFut,
    SFut: Future<Output = Result<Option<EpochSummary>, E>>,
    M: FnOnce(L1BlockCommitment) -> MFut,
    MFut: Future<Output = Result<Option<AsmManifest>, E>>,
{
    let prev_epoch_last_l1_block =
        resolve_prev_epoch_last_l1_block_async(terminal, genesis_l1_block, get_summary).await?;
    let range = EpochL1Range::new(prev_epoch_last_l1_block, epoch_last_l1_block)?;

    // Selection is not async, so look up the one manifest it reads first.
    let manifest = match range.last_processed() {
        Some(&block) => get_manifest(block)
            .await
            .map_err(|source| EpochSpecSelectionError::ManifestLookup { block, source })?,
        None => None,
    };
    Ok(select_next_epoch_spec(spec_versions, range, |_| {
        Ok::<_, E>(manifest)
    })?)
}

/// Returns the last L1 block the epoch before `epoch` processed, which is
/// where `epoch`'s L1 range starts, as [`select_spec_after_epoch`] describes.
fn resolve_prev_epoch_last_l1_block<E>(
    epoch: EpochCommitment,
    genesis_l1_block: L1BlockCommitment,
    mut get_summary: impl FnMut(EpochCommitment) -> Result<Option<EpochSummary>, E>,
) -> Result<L1BlockCommitment, SelectSpecAfterEpochError<E>> {
    if epoch.epoch() == 0 {
        return Ok(genesis_l1_block);
    }
    let mut stored_summary = |epoch| {
        get_summary(epoch)
            .map_err(|source| SelectSpecAfterEpochError::SummaryLookup { epoch, source })?
            .ok_or(SelectSpecAfterEpochError::MissingEpochSummary(epoch))
    };
    let prev_epoch = stored_summary(epoch)?
        .get_prev_epoch_commitment()
        .expect("a summary of an epoch after genesis links to a previous epoch");
    Ok(*stored_summary(prev_epoch)?.new_l1())
}

/// [`resolve_prev_epoch_last_l1_block`] for callers whose summary lookup is
/// async.
async fn resolve_prev_epoch_last_l1_block_async<E, S, SFut>(
    epoch: EpochCommitment,
    genesis_l1_block: L1BlockCommitment,
    mut get_summary: S,
) -> Result<L1BlockCommitment, SelectSpecAfterEpochError<E>>
where
    S: FnMut(EpochCommitment) -> SFut,
    SFut: Future<Output = Result<Option<EpochSummary>, E>>,
{
    if epoch.epoch() == 0 {
        return Ok(genesis_l1_block);
    }
    let summary = get_summary(epoch)
        .await
        .map_err(|source| SelectSpecAfterEpochError::SummaryLookup { epoch, source })?
        .ok_or(SelectSpecAfterEpochError::MissingEpochSummary(epoch))?;
    let prev_epoch = summary
        .get_prev_epoch_commitment()
        .expect("a summary of an epoch after genesis links to a previous epoch");
    let prev_summary = get_summary(prev_epoch)
        .await
        .map_err(|source| SelectSpecAfterEpochError::SummaryLookup {
            epoch: prev_epoch,
            source,
        })?
        .ok_or(SelectSpecAfterEpochError::MissingEpochSummary(prev_epoch))?;
    Ok(*prev_summary.new_l1())
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, future::Ready};

    use futures::executor::block_on;
    use strata_identifiers::{Buf32, L1BlockId, OLBlockCommitment, OLBlockId};
    use strata_ol_stf_v1::test_utils::{
        make_checkpoint_predicate_enactment_manifest, make_empty_manifest,
    };

    use super::*;

    fn block(slot: u64, byte: u8) -> OLBlockCommitment {
        OLBlockCommitment::new(slot, OLBlockId::from(Buf32::from([byte; 32])))
    }

    fn l1_block(height: u32, byte: u8) -> L1BlockCommitment {
        L1BlockCommitment::new(height, L1BlockId::from(Buf32::from([byte; 32])))
    }

    fn manifest_block(manifest: &AsmManifest) -> L1BlockCommitment {
        L1BlockCommitment::new(manifest.height(), *manifest.blkid())
    }

    /// Two summaries for epoch 1 on different forks, and the summary of
    /// epoch 2 on the fork whose epoch 1 ends on L1 height 5.
    struct Fork {
        summaries: HashMap<EpochCommitment, EpochSummary>,
        epoch_2: EpochCommitment,
        linked_last_l1: L1BlockCommitment,
    }

    impl Fork {
        fn new() -> Self {
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
            Self {
                summaries: summaries
                    .into_iter()
                    .map(|summary| (summary.get_epoch_commitment(), summary))
                    .collect(),
                epoch_2,
                linked_last_l1,
            }
        }

        fn summary(&self, epoch: EpochCommitment) -> Result<Option<EpochSummary>, ()> {
            Ok(self.summaries.get(&epoch).copied())
        }
    }

    /// Selects the spec after the fork's epoch 2, whose terminal state runs
    /// `spec` and processed L1 blocks up to `manifest`'s.
    fn select_after_epoch_2(
        fork: &Fork,
        spec: OLSpecId,
        manifest: AsmManifest,
    ) -> Result<OLSpecId, SelectSpecAfterEpochError<()>> {
        let expected = manifest_block(&manifest);
        select_spec_after_epoch(
            fork.epoch_2,
            OLSpecVersions::uniform(spec),
            expected,
            l1_block(0, 0),
            |epoch| fork.summary(epoch),
            |block| {
                assert_eq!(block, expected, "looked up another block's manifest");
                Ok(Some(manifest))
            },
        )
    }

    #[test]
    fn test_genesis_epoch_selects_without_lookups() {
        let genesis = EpochCommitment::from_terminal(0, block(0, 0));
        let anchor = l1_block(3, 9);

        let spec = select_spec_after_epoch(
            genesis,
            OLSpecVersions::uniform(OLSpecId::V0),
            anchor,
            anchor,
            |epoch| -> Result<Option<EpochSummary>, ()> {
                panic!("looked up the summary of {epoch} for the genesis epoch")
            },
            |block| panic!("looked up the manifest of {block} for an empty range"),
        );

        assert_eq!(spec.unwrap(), OLSpecId::V0);
    }

    #[test]
    fn test_range_starts_at_the_epochs_own_link_on_a_fork() {
        let fork = Fork::new();
        let start = resolve_prev_epoch_last_l1_block(fork.epoch_2, l1_block(0, 0), |epoch| {
            fork.summary(epoch)
        });

        assert_eq!(start.unwrap(), fork.linked_last_l1);
    }

    #[test]
    fn test_enactment_in_the_epochs_range_selects_the_successor() {
        let fork = Fork::new();
        let enactment = make_checkpoint_predicate_enactment_manifest(9, 1);

        let spec = select_after_epoch_2(&fork, OLSpecId::V0, enactment);

        assert_eq!(spec.unwrap(), OLSpecId::V1);
    }

    #[test]
    fn test_enactment_without_an_implemented_successor_requires_an_upgrade() {
        let fork = Fork::new();
        let enactment = make_checkpoint_predicate_enactment_manifest(9, 1);

        let err = select_after_epoch_2(&fork, OLSpecId::V1, enactment).unwrap_err();

        assert!(
            matches!(
                err,
                SelectSpecAfterEpochError::Selection(EpochSpecSelectionError::UpgradeRequired(_))
            ),
            "{err:?}"
        );
    }

    #[test]
    fn test_plain_manifest_keeps_the_spec() {
        let fork = Fork::new();

        let spec = select_after_epoch_2(&fork, OLSpecId::V1, make_empty_manifest(9, 3));

        assert_eq!(spec.unwrap(), OLSpecId::V1);
    }

    #[test]
    fn test_missing_summary_is_reported() {
        let fork = Fork::new();
        let missing = EpochCommitment::from_terminal(3, block(30, 4));

        let start =
            resolve_prev_epoch_last_l1_block(missing, l1_block(0, 0), |epoch| fork.summary(epoch));

        assert!(
            matches!(start, Err(SelectSpecAfterEpochError::MissingEpochSummary(epoch)) if epoch == missing),
            "{start:?}"
        );
    }

    #[test]
    fn test_missing_linked_summary_is_reported() {
        let mut fork = Fork::new();
        let linked = fork.summaries[&fork.epoch_2]
            .get_prev_epoch_commitment()
            .expect("epoch 2 links to epoch 1");
        fork.summaries.remove(&linked);

        let start = resolve_prev_epoch_last_l1_block(fork.epoch_2, l1_block(0, 0), |epoch| {
            fork.summary(epoch)
        });

        assert!(
            matches!(start, Err(SelectSpecAfterEpochError::MissingEpochSummary(epoch)) if epoch == linked),
            "{start:?}"
        );
    }

    #[test]
    fn test_summary_lookup_error_names_the_epoch() {
        let fork = Fork::new();

        let start = resolve_prev_epoch_last_l1_block(fork.epoch_2, l1_block(0, 0), |_| {
            Err::<Option<EpochSummary>, _>(())
        });

        assert!(
            matches!(start, Err(SelectSpecAfterEpochError::SummaryLookup { epoch, .. }) if epoch == fork.epoch_2),
            "{start:?}"
        );
    }

    #[test]
    fn test_async_genesis_epoch_selects_without_lookups() {
        let genesis = EpochCommitment::from_terminal(0, block(0, 0));
        let anchor = l1_block(3, 9);

        let spec = block_on(select_spec_after_epoch_async(
            genesis,
            OLSpecVersions::uniform(OLSpecId::V0),
            anchor,
            anchor,
            |epoch| -> Ready<Result<Option<EpochSummary>, ()>> {
                panic!("looked up the summary of {epoch} for the genesis epoch")
            },
            |block| -> Ready<Result<Option<AsmManifest>, ()>> {
                panic!("looked up the manifest of {block} for an empty range")
            },
        ));

        assert_eq!(spec.unwrap(), OLSpecId::V0);
    }

    #[test]
    fn test_async_selection_follows_the_epochs_own_link_on_a_fork() {
        let fork = Fork::new();
        let enactment = make_checkpoint_predicate_enactment_manifest(9, 1);
        let expected = manifest_block(&enactment);

        let spec = block_on(select_spec_after_epoch_async(
            fork.epoch_2,
            OLSpecVersions::uniform(OLSpecId::V0),
            expected,
            l1_block(0, 0),
            |epoch| {
                let summary = fork.summary(epoch);
                async move { summary }
            },
            |block| {
                assert_eq!(block, expected, "looked up another block's manifest");
                async move { Ok(Some(enactment)) }
            },
        ));

        assert_eq!(spec.unwrap(), OLSpecId::V1);
    }

    #[test]
    fn test_async_manifest_lookup_error_names_the_block() {
        let fork = Fork::new();
        let last = l1_block(9, 3);

        let err = block_on(select_spec_after_epoch_async(
            fork.epoch_2,
            OLSpecVersions::uniform(OLSpecId::V1),
            last,
            l1_block(0, 0),
            |epoch| {
                let summary = fork.summary(epoch);
                async move { summary }
            },
            |_| async { Err::<Option<AsmManifest>, _>(()) },
        ))
        .unwrap_err();

        assert!(
            matches!(
                err,
                SelectSpecAfterEpochError::Selection(EpochSpecSelectionError::ManifestLookup {
                    block,
                    ..
                }) if block == last
            ),
            "{err:?}"
        );
    }
}
