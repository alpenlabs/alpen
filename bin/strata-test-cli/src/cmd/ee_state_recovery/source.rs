//! Reads ordered EE DA transaction references from the stopped source sequencer database.

use std::{collections::BTreeMap, path::Path};

use alpen_ee_database::init_db_storage;
use anyhow::{anyhow, bail, Context, Result};
use bitcoin::{hashes::Hash as _, Txid};
use strata_db_types::common::L1TxId;
use tokio::runtime::Handle;

use super::reconstruct::{BatchManifest, ReplayManifest};

fn to_bitcoin_txid(txid: L1TxId) -> Txid {
    Txid::from_byte_array(txid.0)
}

fn take_ordered_batches(
    mut batches_by_sequence: BTreeMap<u64, BatchManifest>,
    target_update_seq_no: u64,
) -> Result<Vec<BatchManifest>> {
    let mut batches = Vec::new();
    for sequence in 0..=target_update_seq_no {
        let batch = batches_by_sequence.remove(&sequence).with_context(|| {
            format!("missing source DA transaction reference for sequence {sequence}")
        })?;
        batches.push(batch);
    }
    Ok(batches)
}

/// Reads envelope transaction IDs in their source ordering.
pub(super) async fn load_source(
    source_datadir: &Path,
    target_update_seq_no: u64,
) -> Result<ReplayManifest> {
    let databases = init_db_storage(source_datadir, 5).map_err(|error| {
        anyhow!(
            "opening source EE datadir {}: {error:#}",
            source_datadir.display()
        )
    })?;
    let handle = Handle::current();
    let envelope_ops = databases.chunked_envelope_ops(handle);

    let next_envelope_idx: u64 = envelope_ops
        .get_next_chunked_envelope_idx_async()
        .await
        .context("reading source envelope frontier")?;
    let entry_limit =
        usize::try_from(next_envelope_idx).context("source envelope frontier exceeds usize")?;
    let mut entries = envelope_ops
        .get_chunked_envelope_entries_from_async(0, entry_limit)
        .await
        .context("reading source envelope entries")?;
    entries.sort_by_key(|(envelope_idx, _)| *envelope_idx);

    let mut batches_by_sequence = BTreeMap::new();
    for (sequence, (envelope_idx, entry)) in entries.into_iter().enumerate() {
        let sequence = u64::try_from(sequence).context("source envelope sequence overflow")?;
        if sequence > target_update_seq_no {
            continue;
        }
        if entry.commit_txid == L1TxId::zero() || entry.reveals.is_empty() {
            bail!("source envelope {envelope_idx} has no signed commit/reveal transaction IDs");
        }

        let batch = BatchManifest {
            update_seq_no: sequence,
            commit_txid: to_bitcoin_txid(entry.commit_txid),
            reveal_txids: entry
                .reveals
                .iter()
                .map(|reveal| to_bitcoin_txid(reveal.txid))
                .collect(),
        };
        if batches_by_sequence.insert(sequence, batch).is_some() {
            bail!("duplicate source envelope ordering position {sequence}");
        }
    }

    let batches = take_ordered_batches(batches_by_sequence, target_update_seq_no)?;

    Ok(ReplayManifest {
        batches,
        raw_transactions: BTreeMap::new(),
    })
}

#[cfg(test)]
mod tests {
    use std::{array, collections::BTreeMap};

    use strata_db_types::common::L1TxId;

    use super::{take_ordered_batches, to_bitcoin_txid, BatchManifest, ReplayManifest};

    fn batch(update_seq_no: u64) -> BatchManifest {
        BatchManifest {
            update_seq_no,
            commit_txid: to_bitcoin_txid(L1TxId::from([update_seq_no as u8; 32])),
            reveal_txids: vec![to_bitcoin_txid(L1TxId::from(
                [update_seq_no.wrapping_add(1) as u8; 32],
            ))],
        }
    }

    #[test]
    fn serializes_complete_bitcoin_txids() {
        let stored_txid = L1TxId::from(array::from_fn(|idx| idx as u8));
        let bitcoin_txid = to_bitcoin_txid(stored_txid);
        let manifest = ReplayManifest {
            batches: vec![BatchManifest {
                update_seq_no: 0,
                commit_txid: bitcoin_txid,
                reveal_txids: vec![bitcoin_txid],
            }],
            raw_transactions: BTreeMap::from([(bitcoin_txid, "00".to_string())]),
        };

        let json = serde_json::to_value(&manifest).unwrap();
        let commit_txid = json["batches"][0]["commit_txid"].as_str().unwrap();

        assert_eq!(commit_txid, bitcoin_txid.to_string());
        assert_eq!(commit_txid, format!("{stored_txid:?}"));
        assert_eq!(commit_txid.len(), 64);
        assert!(!commit_txid.contains(".."));
        assert!(json["raw_transactions"].get(commit_txid).is_some());

        let decoded: ReplayManifest = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.batches[0].commit_txid, bitcoin_txid);
        assert_eq!(decoded.batches[0].reveal_txids, [bitcoin_txid]);
        assert_eq!(decoded.raw_transactions[&bitcoin_txid], "00");
    }

    #[test]
    fn orders_complete_prefix_by_sequence() {
        let input = BTreeMap::from([(1, batch(1)), (0, batch(0)), (2, batch(2))]);

        let ordered = take_ordered_batches(input, 2).unwrap();

        assert_eq!(
            ordered
                .iter()
                .map(|entry| entry.update_seq_no)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn rejects_gap_before_target() {
        let input = BTreeMap::from([(0, batch(0)), (2, batch(2))]);

        let error = take_ordered_batches(input, 2).unwrap_err();

        assert!(error.to_string().contains("sequence 1"));
    }
}
