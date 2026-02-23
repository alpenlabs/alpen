//! [`JsonSchema`](schemars::JsonSchema) implementations for OL RPC types.

use std::borrow::Cow;

use crate::OLBlockOrTag;

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
