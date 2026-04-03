//! [`JsonSchema`](schemars::JsonSchema) implementations for OL RPC types.

use std::borrow::Cow;

use crate::{OLBlockOrTag, RpcCheckpointConfStatus, RpcCheckpointInfo, RpcCheckpointL1Ref};

impl schemars::JsonSchema for OLBlockOrTag {
    fn schema_name() -> Cow<'static, str> {
        "OLBlockOrTag".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Block identifier: 'latest', 'confirmed', 'finalized', a slot number, or a 0x-prefixed block hash"
        })
    }
}

impl schemars::JsonSchema for RpcCheckpointConfStatus {
    fn schema_name() -> Cow<'static, str> {
        "RpcCheckpointConfStatus".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "oneOf": [
                {
                    "type": "object",
                    "required": ["status"],
                    "properties": {
                        "status": { "const": "pending" }
                    }
                },
                {
                    "type": "object",
                    "required": ["status", "l1_reference"],
                    "properties": {
                        "status": { "const": "confirmed" },
                        "l1_reference": { "type": "object" }
                    }
                },
                {
                    "type": "object",
                    "required": ["status", "l1_reference"],
                    "properties": {
                        "status": { "const": "finalized" },
                        "l1_reference": { "type": "object" }
                    }
                }
            ]
        })
    }
}

impl schemars::JsonSchema for RpcCheckpointL1Ref {
    fn schema_name() -> Cow<'static, str> {
        "RpcCheckpointL1Ref".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Keep schema permissive here because L1 commitment wrappers do not
        // currently implement JsonSchema in strata-identifiers.
        schemars::json_schema!({
            "type": "object",
            "required": ["l1_block", "txid", "wtxid"],
            "properties": {
                "l1_block": { "type": "object" },
                "txid": { "type": "string" },
                "wtxid": { "type": "string" }
            }
        })
    }
}

impl schemars::JsonSchema for RpcCheckpointInfo {
    fn schema_name() -> Cow<'static, str> {
        "RpcCheckpointInfo".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        // Keep schema permissive for commitment wrappers not exposing JsonSchema.
        schemars::json_schema!({
            "type": "object",
            "required": [
                "idx",
                "l1_range",
                "l2_range",
                "confirmation_status"
            ],
            "properties": {
                "idx": { "type": "integer", "minimum": 0 },
                "l1_range": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 2,
                    "items": { "type": "object" }
                },
                "l2_range": {
                    "type": "array",
                    "minItems": 2,
                    "maxItems": 2,
                    "items": { "type": "object" }
                },
                "confirmation_status": {
                    "type": "object"
                }
            }
        })
    }
}
