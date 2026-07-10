//! Alpen OpenRPC specification assembly.

use alpen_ee_rpc_api::AlpenEeRpcOpenRpc;
use strata_ol_rpc_api::{
    OLClientRpcOpenRpc, OLFullNodeRpcOpenRpc, OLSequencerRpcOpenRpc, OLSubmitRpcOpenRpc,
};
use strata_open_rpc::Project;

/// Builds the combined OpenRPC document for Alpen binary RPC methods.
pub fn alpen_rpc_project() -> Project {
    let mut project = Project::new(
        "0.4.0",
        "Alpen RPC",
        "Alpen JSON-RPC API",
        "Alpen Labs",
        "https://alpenlabs.io",
        "",
        "MIT",
        "",
    );

    project.add_module(OLFullNodeRpcOpenRpc::module_doc());
    project.add_module(OLClientRpcOpenRpc::module_doc());
    project.add_module(OLSequencerRpcOpenRpc::module_doc());
    project.add_module(OLSubmitRpcOpenRpc::module_doc());
    project.add_module(AlpenEeRpcOpenRpc::module_doc());

    project
}

/// Serializes the combined Alpen OpenRPC document.
pub fn serialize_alpen_rpc_project(compact: bool) -> Result<String, serde_json::Error> {
    let project = alpen_rpc_project();

    if compact {
        serde_json::to_string(&project)
    } else {
        serde_json::to_string_pretty(&project)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::alpen_rpc_project;

    #[test]
    fn includes_ol_and_alpen_ee_methods() {
        let project = alpen_rpc_project();
        let json = serde_json::to_value(project).expect("serialization should not fail");
        let methods = json
            .get("methods")
            .and_then(Value::as_array)
            .expect("project should include methods");

        let method_names = methods
            .iter()
            .filter_map(|method| method.get("name"))
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();

        for expected_method in [
            "strata_submitTransaction",
            "alpen_getBlockStatus",
            "alpen_getChunkProofCoverage",
        ] {
            assert!(
                method_names.contains(&expected_method),
                "expected OpenRPC spec to include {expected_method}"
            );
        }
    }
}
