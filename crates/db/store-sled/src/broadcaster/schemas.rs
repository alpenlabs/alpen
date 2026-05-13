use std::io::Error;

use borsh::BorshDeserialize;
#[cfg(test)]
use borsh::BorshSerialize;
use strata_db_types::types::{L1TxEntry, L1TxStatus};
use strata_primitives::buf::Buf32;
use typed_sled::codec::{CodecError, ValueCodec};

use crate::{define_table_with_integer_key, define_table_without_codec, impl_borsh_key_codec};

define_table_with_integer_key!(
    /// A table to store mapping of idx to L1 txid
    (BcastL1TxIdSchema) u64 => Buf32
);

define_table_without_codec!(
    /// A table to store L1 txs
    (BcastL1TxSchema) Buf32 => L1TxEntry
);

impl_borsh_key_codec!(BcastL1TxSchema, Buf32);

impl ValueCodec<BcastL1TxSchema> for L1TxEntry {
    type Decoded = Self;

    fn encode_value(&self) -> Result<Vec<u8>, CodecError> {
        let mut buf = Vec::new();
        ciborium::into_writer(self, &mut buf).map_err(|err| CodecError::SerializationFailed {
            schema: BcastL1TxSchema::tree_name(),
            source: Box::new(err),
        })?;
        Ok(buf)
    }

    fn decode_value(data: sled::IVec) -> Result<Self::Decoded, CodecError> {
        if let Ok(entry) = ciborium::from_reader(data.as_ref()) {
            return Ok(entry);
        }

        decode_legacy_l1_tx_entry(data.as_ref()).map_err(|err| CodecError::DeserializationFailed {
            schema: BcastL1TxSchema::tree_name(),
            source: Box::new(err),
        })
    }
}

#[derive(BorshDeserialize)]
#[cfg_attr(test, derive(BorshSerialize))]
struct LegacyL1TxEntry {
    tx_raw: Vec<u8>,
    status: L1TxStatus,
}

fn decode_legacy_l1_tx_entry(data: &[u8]) -> Result<L1TxEntry, Error> {
    let legacy = LegacyL1TxEntry::deserialize_reader(&mut &data[..])?;
    Ok(L1TxEntry::from_raw_parts(
        legacy.tx_raw,
        legacy.status,
        None,
    ))
}

#[cfg(test)]
mod tests {
    use strata_db_types::types::L1TxRbfInfo;

    use super::*;

    #[test]
    fn bcast_l1_tx_schema_decodes_legacy_borsh_entries() {
        let legacy = LegacyL1TxEntry {
            tx_raw: vec![1, 2, 3],
            status: L1TxStatus::Published,
        };
        let bytes = borsh::to_vec(&legacy).unwrap();

        let decoded =
            <L1TxEntry as ValueCodec<BcastL1TxSchema>>::decode_value(sled::IVec::from(bytes))
                .unwrap();

        assert_eq!(decoded.tx_raw(), &[1, 2, 3]);
        assert_eq!(decoded.status, L1TxStatus::Published);
        assert_eq!(decoded.rbf, None);
    }

    #[test]
    fn bcast_l1_tx_schema_cbor_roundtrip_preserves_rbf_metadata() {
        let entry = L1TxEntry::from_raw_parts(
            vec![1, 2, 3],
            L1TxStatus::Published,
            Some(L1TxRbfInfo {
                first_published_l1_height: Some(42),
                fee_rate_sat_vb: 7,
                bump_count: 2,
            }),
        );

        let bytes = <L1TxEntry as ValueCodec<BcastL1TxSchema>>::encode_value(&entry).unwrap();
        let decoded =
            <L1TxEntry as ValueCodec<BcastL1TxSchema>>::decode_value(sled::IVec::from(bytes))
                .unwrap();

        assert_eq!(decoded, entry);
    }
}
