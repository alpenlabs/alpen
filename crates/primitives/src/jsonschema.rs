//! [`JsonSchema`](schemars::JsonSchema) implementations for primitive types.

use std::borrow::Cow;

use crate::{HexBytes, HexBytes32, HexBytes64};

impl schemars::JsonSchema for HexBytes {
    fn schema_name() -> Cow<'static, str> {
        "HexBytes".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "hex",
            "description": "Hex-encoded byte array"
        })
    }
}

impl schemars::JsonSchema for HexBytes32 {
    fn schema_name() -> Cow<'static, str> {
        "HexBytes32".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "hex",
            "description": "32-byte hex-encoded value"
        })
    }
}

impl schemars::JsonSchema for HexBytes64 {
    fn schema_name() -> Cow<'static, str> {
        "HexBytes64".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "format": "hex",
            "description": "64-byte hex-encoded value"
        })
    }
}
