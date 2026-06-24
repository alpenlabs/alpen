//! Alpen EE RPC API definitions.

use std::borrow::Cow;

use alloy_primitives::B256;
pub use alpen_ee_rpc_types::{BlockStatus, BlockStatusResponse, ChunkProofCoverageResponse};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// RPC methods exposed by Alpen EE nodes.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "alpen"))]
#[cfg_attr(feature = "client", rpc(server, client, namespace = "alpen"))]
pub trait AlpenEeRpc {
    /// Returns the L1 finalization status for an EE block.
    #[method(name = "getBlockStatus")]
    async fn get_block_status(&self, block_hash: B256) -> RpcResult<BlockStatusResponse>;

    /// Reports whether proof-ready chunks cover the requested EE block interval.
    #[method(name = "getChunkProofCoverage")]
    async fn get_chunk_proof_coverage(
        &self,
        start_block: u64,
        end_block: u64,
    ) -> RpcResult<ChunkProofCoverageResponse>;
}

struct RpcB256;

impl schemars::JsonSchema for RpcB256 {
    fn schema_name() -> Cow<'static, str> {
        "B256".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^0x[0-9a-fA-F]{64}$",
            "description": "32-byte 0x-prefixed hex string"
        })
    }
}

/// OpenRPC documentation for the Alpen EE RPC API.
#[derive(Debug)]
pub struct AlpenEeRpcOpenRpc;

impl AlpenEeRpcOpenRpc {
    pub fn module_doc() -> strata_open_rpc::Module {
        let mut builder = strata_open_rpc::RpcModuleDocBuilder::default();

        let inputs =
            vec![builder.create_content_descriptor::<RpcB256>("block_hash", None, None, true)];
        let result = Some(builder.create_content_descriptor::<BlockStatusResponse>(
            "BlockStatusResponse",
            None,
            None,
            true,
        ));
        builder.add_method(
            "alpen",
            "getBlockStatus",
            inputs,
            result,
            "Returns the L1 finalization status for an EE block.",
            Some("Alpen EE".to_string()),
            false,
        );

        let inputs = vec![
            builder.create_content_descriptor::<u64>("start_block", None, None, true),
            builder.create_content_descriptor::<u64>("end_block", None, None, true),
        ];
        let result = Some(
            builder.create_content_descriptor::<ChunkProofCoverageResponse>(
                "ChunkProofCoverageResponse",
                None,
                None,
                true,
            ),
        );
        builder.add_method(
            "alpen",
            "getChunkProofCoverage",
            inputs,
            result,
            "Reports whether proof-ready chunks cover the requested EE block interval.",
            Some("Alpen EE".to_string()),
            false,
        );

        builder.build()
    }
}
