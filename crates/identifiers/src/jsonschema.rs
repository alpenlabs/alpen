//! [`JsonSchema`](schemars::JsonSchema) implementations for identifier types.

use std::borrow::Cow;

use crate::{
    AccountId, Buf32, Buf64, Epoch, EpochCommitment, OLBlockCommitment, OLBlockId, OLTxId, Slot,
};

impl schemars::JsonSchema for Buf32 {
    fn schema_name() -> Cow<'static, str> {
        "Buf32".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "hex",
            "description": "32-byte hex-encoded value"
        })
    }
}

impl schemars::JsonSchema for Buf64 {
    fn schema_name() -> Cow<'static, str> {
        "Buf64".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "hex",
            "description": "64-byte hex-encoded value"
        })
    }
}

impl schemars::JsonSchema for AccountId {
    fn schema_name() -> Cow<'static, str> {
        "AccountId".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "hex",
            "description": "32-byte hex-encoded account identifier"
        })
    }
}

impl schemars::JsonSchema for EpochCommitment {
    fn schema_name() -> Cow<'static, str> {
        "EpochCommitment".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let epoch_schema = generator.subschema_for::<Epoch>();
        let last_slot_schema = generator.subschema_for::<Slot>();
        let last_blkid_schema = generator.subschema_for::<OLBlockId>();
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "epoch": epoch_schema,
                "last_slot": last_slot_schema,
                "last_blkid": last_blkid_schema
            },
            "required": ["epoch", "last_slot", "last_blkid"]
        })
    }
}

impl schemars::JsonSchema for OLBlockId {
    fn schema_name() -> Cow<'static, str> {
        "OLBlockId".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<Buf32>()
    }
}

impl schemars::JsonSchema for OLBlockCommitment {
    fn schema_name() -> Cow<'static, str> {
        "OLBlockCommitment".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let slot_schema = generator.subschema_for::<u64>();
        let blkid_schema = generator.subschema_for::<OLBlockId>();
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "slot": slot_schema,
                "blkid": blkid_schema
            },
            "required": ["slot", "blkid"]
        })
    }
}

impl schemars::JsonSchema for OLTxId {
    fn schema_name() -> Cow<'static, str> {
        "OLTxId".into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        generator.subschema_for::<Buf32>()
    }
}
