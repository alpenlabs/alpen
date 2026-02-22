//! Generate the Alpen OL OpenRPC specification.
//!
//! Usage:
//!
//! ```sh
//! # Pretty-printed to stdout
//! cargo run -p strata-ol-rpc-api --example openrpc_spec
//!
//! # Write to file
//! cargo run -p strata-ol-rpc-api --example openrpc_spec > ol-openrpc.json
//!
//! # Compact (single line)
//! cargo run -p strata-ol-rpc-api --example openrpc_spec -- --compact
//! ```
//!
//! The output is a valid OpenRPC 1.2.6 document that can be loaded into
//! <https://playground.open-rpc.org> for interactive exploration.
#![allow(unused_crate_dependencies, reason = "example binary")]

use std::env;

use strata_ol_rpc_api::{OLClientRpcOpenRpc, OLFullNodeRpcOpenRpc, OLSequencerRpcOpenRpc};

fn main() {
    let compact = env::args().any(|a| a == "--compact");

    let mut project = strata_open_rpc::Project::new(
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
    project.add_module(OLClientRpcOpenRpc::module_doc());
    project.add_module(OLSequencerRpcOpenRpc::module_doc());

    let json = if compact {
        serde_json::to_string(&project).expect("serialization should not fail")
    } else {
        serde_json::to_string_pretty(&project).expect("serialization should not fail")
    };

    println!("{json}");
}
