use std::collections::BTreeMap;

use strata_asm_common::{
    AsmLogEntry, AuxResolveError, AuxResolveResult, AuxResolver, AuxResponseKind, HistoryMmr,
    HistoryMmrState, L1TxIndex, SubprotocolId, compute_log_leaf,
};

use crate::{AuxResponseEnvelope, HistoricalLogSegment};

/// Resolver that scopes auxiliary responses to a single subprotocol.
#[derive(Debug)]
pub struct SubprotocolAuxResolver<'a> {
    subprotocol: SubprotocolId,
    responses: Option<&'a BTreeMap<L1TxIndex, Vec<AuxResponseEnvelope>>>,
    history_mmr: HistoryMmr,
}

impl<'a> SubprotocolAuxResolver<'a> {
    /// Constructs a resolver for `subprotocol` backed by the global aux response map.
    pub fn new(
        subprotocol: SubprotocolId,
        history_mmr: &HistoryMmrState,
        all_responses: &'a BTreeMap<SubprotocolId, BTreeMap<L1TxIndex, Vec<AuxResponseEnvelope>>>,
    ) -> Self {
        let responses = all_responses.get(&subprotocol);
        Self {
            subprotocol,
            responses,
            history_mmr: HistoryMmr::from_compact(history_mmr.as_compact()),
        }
    }

    fn envelopes_for_tx(&self, tx_index: L1TxIndex) -> Option<&'a [AuxResponseEnvelope]> {
        self.responses
            .and_then(|map| map.get(&tx_index))
            .map(|entries| entries.as_slice())
    }

    fn extract_logs_from_segment(
        &self,
        tx_index: L1TxIndex,
        segment: &HistoricalLogSegment,
    ) -> AuxResolveResult<Vec<AsmLogEntry>> {
        let leaf = compute_log_leaf(&segment.block_hash, &segment.logs);
        if !self.history_mmr.verify(&segment.proof, &leaf) {
            return Err(AuxResolveError::InvalidLogProof {
                subprotocol: self.subprotocol,
                tx_index,
                block_hash: segment.block_hash,
            });
        }

        Ok(segment.logs.to_vec())
    }

    fn extract_logs_from_segments(
        &self,
        tx_index: L1TxIndex,
        segments: &[HistoricalLogSegment],
    ) -> AuxResolveResult<Vec<AsmLogEntry>> {
        let mut logs = Vec::new();
        for segment in segments {
            logs.extend(self.extract_logs_from_segment(tx_index, segment)?);
        }
        Ok(logs)
    }
}

impl AuxResolver for SubprotocolAuxResolver<'_> {
    fn historical_logs(&self, tx_index: L1TxIndex) -> AuxResolveResult<Vec<AsmLogEntry>> {
        let Some(envelopes) = self.envelopes_for_tx(tx_index) else {
            return Ok(Vec::new());
        };

        let mut logs = Vec::new();
        for envelope in envelopes {
            match envelope {
                AuxResponseEnvelope::HistoricalLogs { segments: segment } => {
                    logs.extend(self.extract_logs_from_segment(tx_index, segment)?);
                }
                AuxResponseEnvelope::HistoricalLogsRange { segments } => {
                    logs.extend(self.extract_logs_from_segments(tx_index, segments.as_slice())?);
                }
                AuxResponseEnvelope::DepositRequestTx { .. } => {
                    return Err(AuxResolveError::UnexpectedResponseVariant {
                        subprotocol: self.subprotocol,
                        tx_index,
                        expected: AuxResponseKind::HistoricalLogs,
                        actual: AuxResponseKind::DepositRequestTx,
                    });
                }
            }
        }

        Ok(logs)
    }

    fn deposit_request_tx(&self, tx_index: L1TxIndex) -> AuxResolveResult<Option<Vec<u8>>> {
        let Some(envelopes) = self.envelopes_for_tx(tx_index) else {
            return Ok(None);
        };

        for envelope in envelopes {
            if let AuxResponseEnvelope::DepositRequestTx { raw_tx } = envelope {
                return Ok(Some(raw_tx.clone()));
            }
        }

        Ok(None)
    }
}
