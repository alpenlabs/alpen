//! Read-only commands for inspecting locally produced EE DA blobs.
//!
//! The inspection flow decodes chunked-envelope records from the alpen-client
//! sled database, verifies the ALPN DA blob format, selects the blob that covers
//! a target EVM block, and replays the canonical DA prefix to report the
//! reconstructed post-state root.

use std::collections::{btree_map::Entry, BTreeMap};

use alpen_ee_da_types::{reassemble_da_blob, DA_BLOB_VERSION, EE_DA_MAGIC_BYTES};
use alpen_reth_statediff::{
    apply_batch_state_diff_to_ethereum_state, ethereum_state_from_chain_spec, BatchStateDiff,
};
use argh::FromArgs;
use sha2::{Digest, Sha256};
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_db_store_sled::chunked_envelope::L1ChunkedEnvelopeDBSled;
use strata_db_types::chunked_envelope::L1ChunkedEnvelopeDatabase;

use crate::{
    cli::OutputFormat,
    output::{
        ee_da::{EeDaInspectInfo, EeDaReplayInfo, EeDaTargetInfo},
        output,
    },
};

/// Maximum number of chunked-envelope entries to read from sled per batch.
const ENVELOPE_SCAN_BATCH_SIZE: usize = 128;

/// Arguments for the `ee-da-inspect` dbtool subcommand.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "ee-da-inspect")]
pub(crate) struct EeDaInspectArgs {
    /// chain name or JSON chain spec used to seed the state reconstructor.
    #[argh(option, default = "String::from(\"dev\")")]
    pub(crate) chain: String,

    /// EVM block number that must be covered by the selected DA blob.
    #[argh(option)]
    pub(crate) target_last_block: u64,

    /// output format: "porcelain" (default) or "json".
    #[argh(option, short = 'o', default = "OutputFormat::Porcelain")]
    pub(crate) output_format: OutputFormat,
}

/// Decoded chunked-envelope entry with the fields needed for inspection.
#[derive(Debug)]
struct DecodedEnvelope {
    /// Index of the chunked-envelope record in the EE sled store.
    envelope_idx: u64,
    /// Monotonic DA update sequence number encoded in the blob.
    update_seq_no: u64,
    /// Last EVM block covered by the blob's state diff.
    last_block_num: u64,
    /// Producer-local encoded blob bytes formed by concatenating stored chunks.
    local_blob: Vec<u8>,
    /// Number of chunks stored for the encoded blob.
    chunk_count: usize,
    /// State diff replayed to reconstruct the post-state root.
    state_diff: BatchStateDiff,
}

/// Inspects EE DA records and writes the selected blob plus replay result.
pub(crate) fn ee_da_inspect(
    db: &L1ChunkedEnvelopeDBSled,
    args: EeDaInspectArgs,
) -> Result<(), DisplayedError> {
    let envelopes = load_decoded_envelopes(db, args.target_last_block)?;
    let info = inspect_envelopes(&envelopes, &args.chain, args.target_last_block)?;
    output(&info, args.output_format)
}

/// Loads and validates chunked-envelope DA entries up to the target block.
///
/// The scan walks entries in ascending index order and stops at the first
/// envelope whose `last_block_num` covers `target_last_block`. EE DA envelopes
/// are append-only in DA-update order, so `envelope_idx`, `update_seq_no`, and
/// `last_block_num` advance together; that first covering envelope is the
/// minimal covering blob and `[0..=that]` is the full replay prefix.
///
/// Validation (magic / version / decode) and the downstream continuity checks
/// therefore apply only to this bounded prefix. Entries *after* the first
/// covering envelope are intentionally never decoded, validated, or processed —
/// a malformed or duplicate trailing row cannot change the reconstructed state
/// root for the requested block. Reads are batched, so the final batch may fetch
/// a few rows past the target into memory, but at most one batch (rather than the
/// whole DA history) is ever over-read. When no envelope covers the target the
/// scan reaches the end and the caller reports the "no blob covers" error.
fn load_decoded_envelopes(
    db: &impl L1ChunkedEnvelopeDatabase,
    target_last_block: u64,
) -> Result<Vec<DecodedEnvelope>, DisplayedError> {
    let next_idx = db
        .get_next_chunked_envelope_idx()
        .internal_error("Failed to read next chunked envelope index")?;

    let mut decoded = Vec::new();
    let mut cursor = 0;
    'scan: while cursor < next_idx {
        let remaining = (next_idx - cursor) as usize;
        let batch = db
            .get_chunked_envelope_entries_from(cursor, remaining.min(ENVELOPE_SCAN_BATCH_SIZE))
            .internal_error("Failed to read chunked envelope entries")?;
        if batch.is_empty() {
            break;
        }

        for (idx, entry) in batch {
            if *entry.magic_bytes.as_bytes() != EE_DA_MAGIC_BYTES {
                return Err(DisplayedError::UserError(
                    format!("Chunked envelope {idx} has unexpected magic bytes (expected ALPN)"),
                    Box::new(entry.magic_bytes),
                ));
            }
            if entry.da_blob_version != DA_BLOB_VERSION {
                return Err(DisplayedError::UserError(
                    format!(
                        "Chunked envelope {idx} has unsupported DA blob version \
                         (expected {DA_BLOB_VERSION})"
                    ),
                    Box::new(entry.da_blob_version),
                ));
            }

            let blob = reassemble_da_blob(&entry.chunk_data).map_err(|e| {
                DisplayedError::UserError(
                    format!("Failed to decode DA blob from chunked envelope {idx}"),
                    Box::new(e),
                )
            })?;
            let local_blob = entry.chunk_data.concat();
            let last_block_num = blob.evm_header.block_num;
            decoded.push(DecodedEnvelope {
                envelope_idx: idx,
                update_seq_no: blob.update_seq_no,
                last_block_num,
                local_blob,
                chunk_count: entry.chunk_data.len(),
                state_diff: blob.state_diff,
            });

            // Stop once the target block is covered: the bounded prefix above is
            // everything the replay needs, so the rest of the DA history is left
            // untouched.
            if last_block_num >= target_last_block {
                break 'scan;
            }
        }

        cursor = decoded
            .last()
            .map(|entry| entry.envelope_idx + 1)
            .unwrap_or(cursor + ENVELOPE_SCAN_BATCH_SIZE as u64);
    }

    Ok(decoded)
}

/// Selects the target blob, verifies replay continuity, and builds output.
fn inspect_envelopes(
    envelopes: &[DecodedEnvelope],
    chain: &str,
    target_last_block: u64,
) -> Result<EeDaInspectInfo, DisplayedError> {
    let target = envelopes
        .iter()
        .filter(|entry| entry.last_block_num >= target_last_block)
        .min_by_key(|entry| (entry.last_block_num, entry.update_seq_no))
        .ok_or_else(|| {
            DisplayedError::UserError(
                "No EE DA blob covers the target EVM block".to_string(),
                Box::new(target_last_block),
            )
        })?;

    let replay_entries = ordered_prefix(envelopes, target.update_seq_no)?;
    let post_state_root = replay_state_root(chain, &replay_entries)?;
    let local_blob_sha256 = Sha256::digest(&target.local_blob);

    Ok(EeDaInspectInfo {
        target: EeDaTargetInfo {
            envelope_idx: target.envelope_idx,
            update_seq_no: target.update_seq_no,
            last_block_num: target.last_block_num,
            local_blob_hex: hex::encode(&target.local_blob),
            local_blob_sha256: hex::encode(local_blob_sha256),
            chunk_count: target.chunk_count,
        },
        replay: EeDaReplayInfo { post_state_root },
    })
}

/// Returns the contiguous DA prefix ending at `target_update_seq_no`.
fn ordered_prefix(
    envelopes: &[DecodedEnvelope],
    target_update_seq_no: u64,
) -> Result<Vec<&DecodedEnvelope>, DisplayedError> {
    let mut by_seq_no = BTreeMap::new();
    for envelope in envelopes
        .iter()
        .filter(|entry| entry.update_seq_no <= target_update_seq_no)
    {
        match by_seq_no.entry(envelope.update_seq_no) {
            Entry::Vacant(slot) => {
                slot.insert(envelope);
            }
            Entry::Occupied(existing) => {
                return Err(DisplayedError::UserError(
                    format!(
                        "Duplicate EE DA update_seq_no in envelopes {} and {}",
                        existing.get().envelope_idx,
                        envelope.envelope_idx
                    ),
                    Box::new(envelope.update_seq_no),
                ));
            }
        }
    }

    let mut ordered = Vec::new();
    for expected in 0..=target_update_seq_no {
        let Some(envelope) = by_seq_no.get(&expected).copied() else {
            return Err(DisplayedError::UserError(
                "Missing EE DA update_seq_no before target".to_string(),
                Box::new(expected),
            ));
        };
        ordered.push(envelope);
    }
    Ok(ordered)
}

/// Replays ordered DA state diffs and returns the reconstructed state root.
fn replay_state_root(chain: &str, ordered: &[&DecodedEnvelope]) -> Result<String, DisplayedError> {
    let mut state = ethereum_state_from_chain_spec(chain)
        .internal_error("Failed to initialize Ethereum state from chain spec")?;
    for envelope in ordered {
        apply_batch_state_diff_to_ethereum_state(&mut state, &envelope.state_diff)
            .internal_error("Failed to apply EE DA state diff")?;
    }
    Ok(format!("0x{}", hex::encode(state.state_root().as_slice())))
}

#[cfg(test)]
mod tests {
    //! Unit tests for EE DA target selection and continuity checks.

    use alpen_ee_da_types::{DaBlob, EvmHeaderSummary};
    use strata_codec::encode_to_vec;
    use strata_db_types::{chunked_envelope::ChunkedEnvelopeEntry, DbResult};
    use strata_l1_txfmt::MagicBytes;

    use super::*;

    /// Builds a [`DaBlob`] with an empty state diff for selection tests.
    fn blob(update_seq_no: u64, last_block_num: u64) -> DaBlob {
        DaBlob {
            update_seq_no,
            evm_header: EvmHeaderSummary {
                block_num: last_block_num,
                timestamp: 0,
                base_fee: 0,
                gas_used: 0,
                gas_limit: 30_000_000,
            },
            state_diff: BatchStateDiff::new(),
        }
    }

    /// Builds a decoded envelope with an empty state diff for selection tests.
    fn env(envelope_idx: u64, update_seq_no: u64, last_block_num: u64) -> DecodedEnvelope {
        let blob = blob(update_seq_no, last_block_num);
        DecodedEnvelope {
            envelope_idx,
            update_seq_no,
            last_block_num,
            local_blob: encode_to_vec(&blob).expect("encode blob"),
            chunk_count: 1,
            state_diff: blob.state_diff,
        }
    }

    /// Builds a single-chunk chunked-envelope entry carrying `blob`, tagged with
    /// the given magic bytes so tests can inject a deliberately poisoned row.
    fn entry(update_seq_no: u64, last_block_num: u64, magic: [u8; 4]) -> ChunkedEnvelopeEntry {
        let chunk_data = vec![encode_to_vec(&blob(update_seq_no, last_block_num)).expect("encode")];
        ChunkedEnvelopeEntry::new_unsigned(chunk_data, MagicBytes::new(magic), DA_BLOB_VERSION)
    }

    /// In-memory [`L1ChunkedEnvelopeDatabase`] backing the bounded-scan test.
    ///
    /// Only the two read paths the scan uses are implemented; the rest panic so
    /// an accidental call surfaces loudly.
    struct VecEnvelopeDb {
        entries: Vec<(u64, ChunkedEnvelopeEntry)>,
    }

    impl L1ChunkedEnvelopeDatabase for VecEnvelopeDb {
        fn get_next_chunked_envelope_idx(&self) -> DbResult<u64> {
            Ok(self.entries.last().map(|(idx, _)| idx + 1).unwrap_or(0))
        }

        fn get_chunked_envelope_entries_from(
            &self,
            start_idx: u64,
            max_count: usize,
        ) -> DbResult<Vec<(u64, ChunkedEnvelopeEntry)>> {
            Ok(self
                .entries
                .iter()
                .filter(|(idx, _)| *idx >= start_idx)
                .take(max_count)
                .cloned()
                .collect())
        }

        fn put_chunked_envelope_entry(
            &self,
            _idx: u64,
            _entry: ChunkedEnvelopeEntry,
        ) -> DbResult<()> {
            unimplemented!()
        }

        fn get_chunked_envelope_entry(&self, _idx: u64) -> DbResult<Option<ChunkedEnvelopeEntry>> {
            unimplemented!()
        }

        fn del_chunked_envelope_entry(&self, _idx: u64) -> DbResult<bool> {
            unimplemented!()
        }

        fn del_chunked_envelope_entries_from_idx(&self, _start_idx: u64) -> DbResult<Vec<u64>> {
            unimplemented!()
        }
    }

    /// Selects the first DA blob whose covered block range includes the target.
    #[test]
    fn selects_first_blob_covering_target_block() {
        let envelopes = vec![env(0, 0, 2), env(1, 1, 5), env(2, 2, 9)];
        let info = inspect_envelopes(&envelopes, "dev", 4).expect("inspect");
        assert_eq!(info.target.envelope_idx, 1);
        assert_eq!(info.target.update_seq_no, 1);
        assert_eq!(info.target.last_block_num, 5);
    }

    /// Rejects a replay prefix with a missing update sequence number.
    #[test]
    fn rejects_missing_update_seq_no_before_target() {
        let envelopes = vec![env(0, 0, 2), env(2, 2, 9)];
        let err = inspect_envelopes(&envelopes, "dev", 9).expect_err("gap must fail");
        assert!(err
            .to_string()
            .contains("Missing EE DA update_seq_no before target"));
    }

    /// Rejects duplicate update sequence numbers before the selected target.
    #[test]
    fn rejects_duplicate_update_seq_no_before_target() {
        let envelopes = vec![env(0, 0, 2), env(1, 1, 5), env(2, 1, 6)];
        let err = inspect_envelopes(&envelopes, "dev", 6).expect_err("duplicate must fail");
        assert!(err.to_string().contains("Duplicate EE DA update_seq_no"));
    }

    /// Stops the scan at the first envelope covering the target block, leaving
    /// later history untouched.
    ///
    /// The trailing entry carries poisoned magic bytes that would fail
    /// validation if it were ever decoded, so a clean load proves the scan does
    /// not process entries past the selected target.
    #[test]
    fn load_stops_at_first_covering_envelope() {
        let db = VecEnvelopeDb {
            entries: vec![
                (0, entry(0, 2, EE_DA_MAGIC_BYTES)),
                (1, entry(1, 5, EE_DA_MAGIC_BYTES)),
                (2, entry(2, 9, [0; 4])),
            ],
        };

        let decoded = load_decoded_envelopes(&db, 4).expect("bounded load");
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded.last().expect("covering entry").last_block_num, 5);
    }
}
