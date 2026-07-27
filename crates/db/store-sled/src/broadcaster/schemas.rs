use strata_db_types::fee_bump::{TxNodeId, TxNodeRecord};
use strata_db_types::l1_broadcast::L1TxEntry;
use strata_primitives::buf::Buf32;
use typed_sled::codec::{CodecError, KeyCodec};

use crate::{define_table_without_codec, impl_cbor_value_codec, impl_codec_key_codec};

define_table_without_codec!(
    /// A table to store mapping of idx to L1 txid
    (BcastL1TxIdSchema) u64 => Buf32
);
// `u64` keys use typed-sled's blanket big-endian integer key codec.
impl_cbor_value_codec!(BcastL1TxIdSchema, Buf32);

define_table_without_codec!(
    /// A table to store L1 txs
    (BcastL1TxSchema) Buf32 => L1TxEntry
);
impl_codec_key_codec!(BcastL1TxSchema, Buf32);
impl_cbor_value_codec!(BcastL1TxSchema, L1TxEntry);

define_table_without_codec!(
    /// A table to store logical L1 transaction replacement chains
    (BcastL1TxNodeSchema) TxNodeId => TxNodeRecord
);

define_table_without_codec!(
    /// Presence marker: this replacement chain may still need fee bumping.
    ///
    /// The replacement pass scans this set instead of the full node tree, whose records are kept
    /// forever for crash-recovery point lookups.
    (BcastActiveL1TxNodeSchema) TxNodeId => ()
);

const HASH_KEY_LEN: usize = 32;

fn decode_hash_key(data: &[u8], schema: &'static str) -> Result<Buf32, CodecError> {
    let bytes = data.try_into().map_err(|_| CodecError::InvalidKeyLength {
        schema,
        expected: HASH_KEY_LEN,
        actual: data.len(),
    })?;
    Ok(Buf32(bytes))
}

impl KeyCodec<BcastL1TxNodeSchema> for TxNodeId {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0.0.to_vec())
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        decode_hash_key(data, BcastL1TxNodeSchema::tree_name()).map(Self)
    }
}

impl KeyCodec<BcastActiveL1TxNodeSchema> for TxNodeId {
    fn encode_key(&self) -> Result<Vec<u8>, CodecError> {
        Ok(self.0.0.to_vec())
    }

    fn decode_key(data: &[u8]) -> Result<Self, CodecError> {
        decode_hash_key(data, BcastActiveL1TxNodeSchema::tree_name()).map(Self)
    }
}

impl_cbor_value_codec!(BcastL1TxNodeSchema, TxNodeRecord);
impl_cbor_value_codec!(BcastActiveL1TxNodeSchema, ());

#[cfg(test)]
mod tests {
    use strata_db_types::common::{L1TxId, L1WtxId};
    use strata_db_types::fee_bump::{TxAttempt, TxAttemptStatus, TxNodeKind};
    use strata_db_types::l1_broadcast::{L1TxRbfInfo, L1TxStatus};
    use typed_sled::codec::ValueCodec;

    use super::*;

    #[test]
    fn bcast_l1_tx_schema_decodes_cbor_without_rbf_metadata() {
        let old_shape = ciborium::Value::Map(vec![
            (
                ciborium::Value::Text("tx_raw".into()),
                ciborium::Value::Bytes(vec![1, 2, 3]),
            ),
            (
                ciborium::Value::Text("status".into()),
                ciborium::Value::Map(vec![(
                    ciborium::Value::Text("status".into()),
                    ciborium::Value::Text("published".into()),
                )]),
            ),
        ]);
        let mut bytes = Vec::new();
        ciborium::into_writer(&old_shape, &mut bytes).unwrap();

        let decoded =
            <L1TxEntry as ValueCodec<BcastL1TxSchema>>::decode_value(sled::IVec::from(bytes))
                .unwrap();

        assert_eq!(decoded.tx_raw(), &[1, 2, 3]);
        assert_eq!(decoded.status, L1TxStatus::Published);
        assert_eq!(decoded.rbf, None);
    }

    #[test]
    fn broadcaster_schema_keys_use_raw_hash_bytes() {
        let txid = Buf32([0x11; HASH_KEY_LEN]);
        let txid_bytes = <Buf32 as KeyCodec<BcastL1TxSchema>>::encode_key(&txid).unwrap();
        assert_eq!(txid_bytes, vec![0x11; HASH_KEY_LEN]);
        assert_eq!(
            <Buf32 as KeyCodec<BcastL1TxSchema>>::decode_key(&txid_bytes).unwrap(),
            txid
        );

        let node_id = TxNodeId(Buf32([0x22; HASH_KEY_LEN]));
        let node_id_bytes =
            <TxNodeId as KeyCodec<BcastL1TxNodeSchema>>::encode_key(&node_id).unwrap();
        assert_eq!(node_id_bytes, vec![0x22; HASH_KEY_LEN]);
        assert_eq!(
            <TxNodeId as KeyCodec<BcastL1TxNodeSchema>>::decode_key(&node_id_bytes).unwrap(),
            node_id
        );
    }

    #[test]
    fn broadcaster_schema_keys_reject_invalid_lengths() {
        let txid_err =
            <Buf32 as KeyCodec<BcastL1TxSchema>>::decode_key(&[0; HASH_KEY_LEN - 1]).unwrap_err();
        assert!(matches!(txid_err, CodecError::SerializationFailed { .. }));

        let node_err =
            <TxNodeId as KeyCodec<BcastL1TxNodeSchema>>::decode_key(&[0; HASH_KEY_LEN + 1])
                .unwrap_err();
        assert!(matches!(
            node_err,
            CodecError::InvalidKeyLength {
                expected: HASH_KEY_LEN,
                actual: 33,
                ..
            }
        ));
    }

    #[test]
    fn bcast_l1_tx_schema_cbor_roundtrip_preserves_rbf_metadata() {
        let entry = L1TxEntry::from_raw_parts(
            vec![1, 2, 3],
            L1TxStatus::Published,
            Some(L1TxRbfInfo {
                fee_rate_sat_vb: 7,
                fee_sats: 700,
                replaces: None,
            }),
        );

        let bytes = <L1TxEntry as ValueCodec<BcastL1TxSchema>>::encode_value(&entry).unwrap();
        let decoded =
            <L1TxEntry as ValueCodec<BcastL1TxSchema>>::decode_value(sled::IVec::from(bytes))
                .unwrap();

        assert_eq!(decoded, entry);
    }

    #[test]
    fn bcast_l1_tx_node_schema_cbor_roundtrip() {
        let kind = TxNodeKind::SingleEnvelopeCommit { payload_idx: 42 };
        let attempt = TxAttempt {
            attempt_no: 0,
            raw_tx: vec![1, 2, 3],
            txid: L1TxId::from([1; 32]),
            wtxid: L1WtxId::from([2; 32]),
            fee_rate_sat_vb: 7,
            fee_sats: 700,
            created_at_unix_secs: 123,
            first_published_l1_height: None,
            status: TxAttemptStatus::Active,
            replaced_by: None,
        };
        let record = TxNodeRecord::new(kind, attempt);

        let bytes =
            <TxNodeRecord as ValueCodec<BcastL1TxNodeSchema>>::encode_value(&record).unwrap();
        let decoded = <TxNodeRecord as ValueCodec<BcastL1TxNodeSchema>>::decode_value(
            sled::IVec::from(bytes),
        )
        .unwrap();

        assert_eq!(decoded, record);
    }
}
