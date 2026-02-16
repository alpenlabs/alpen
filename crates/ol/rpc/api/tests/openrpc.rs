#![allow(unused_crate_dependencies, reason = "integration test binary")]

use strata_ol_rpc_api::OLFullNodeRpcOpenRpc;

fn build_spec() -> serde_json::Value {
    let mut project = alpen_open_rpc::Project::new(
        "0.1.0",
        "Alpen OL RPC",
        "Alpen Orchestration Layer JSON-RPC API",
        "Alpen Labs",
        "https://alpenlabs.io",
        "",
        "MIT",
        "",
    );

    project.add_module(OLFullNodeRpcOpenRpc::module_doc());

    serde_json::to_value(&project).expect("spec should serialize to JSON")
}

#[test]
fn spec_contains_expected_methods() {
    let spec = build_spec();
    let methods = spec["methods"]
        .as_array()
        .expect("methods should be an array");

    let method_names: Vec<&str> = methods
        .iter()
        .map(|m| m["name"].as_str().expect("method should have a name"))
        .collect();

    assert!(
        method_names.contains(&"strata_getRawBlocksRange"),
        "missing strata_getRawBlocksRange, found: {method_names:?}"
    );
    assert!(
        method_names.contains(&"strata_getRawBlockById"),
        "missing strata_getRawBlockById, found: {method_names:?}"
    );
}

#[test]
fn methods_have_params_and_result() {
    let spec = build_spec();
    let methods = spec["methods"].as_array().unwrap();

    for method in methods {
        let name = method["name"].as_str().unwrap();

        assert!(
            method["params"].is_array(),
            "method {name} should have params array"
        );
        assert!(
            method["result"].is_object(),
            "method {name} should have a result"
        );
        assert!(
            method["result"]["schema"].is_object(),
            "method {name} result should have a schema"
        );
    }
}

#[test]
fn get_raw_blocks_range_has_two_params() {
    let spec = build_spec();
    let methods = spec["methods"].as_array().unwrap();

    let method = methods
        .iter()
        .find(|m| m["name"] == "strata_getRawBlocksRange")
        .expect("strata_getRawBlocksRange should exist");

    let params = method["params"].as_array().unwrap();
    assert_eq!(params.len(), 2, "getRawBlocksRange should have 2 params");
    assert_eq!(params[0]["name"], "start_height");
    assert_eq!(params[1]["name"], "end_height");
}

#[test]
fn get_raw_block_by_id_has_one_param() {
    let spec = build_spec();
    let methods = spec["methods"].as_array().unwrap();

    let method = methods
        .iter()
        .find(|m| m["name"] == "strata_getRawBlockById")
        .expect("strata_getRawBlockById should exist");

    let params = method["params"].as_array().unwrap();
    assert_eq!(params.len(), 1, "getRawBlockById should have 1 param");
    assert_eq!(params[0]["name"], "block_id");
}
