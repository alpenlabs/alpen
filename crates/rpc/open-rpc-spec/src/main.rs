//! Generate the Alpen OpenRPC specification.
//!
//! Usage:
//!
//! ```sh
//! # Pretty-printed to stdout
//! cargo run -p strata-rpc-openrpc-spec
//!
//! # Write to file
//! cargo run -p strata-rpc-openrpc-spec > alpen-openrpc.json
//!
//! # Compact (single line)
//! cargo run -p strata-rpc-openrpc-spec -- --compact
//! ```
//!
//! The output is a valid OpenRPC 1.2.6 document that can be loaded into
//! <https://playground.open-rpc.org> for interactive exploration.
#![allow(
    unused_crate_dependencies,
    reason = "binary uses package dependencies through the library crate"
)]

use std::env;

use strata_rpc_openrpc_spec::serialize_alpen_rpc_project;

fn main() {
    let compact = env::args().any(|a| a == "--compact");
    let json = serialize_alpen_rpc_project(compact).expect("serialization should not fail");

    println!("{json}");
}
