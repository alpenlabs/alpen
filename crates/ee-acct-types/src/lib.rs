//! Types relating to exec env account structure and updates.
//!
//! This is generally not exposed directly to the orchestration layer, although
//! some messages may be encoded in orchestration layer containers (which are
//! treated opaquely).

mod commit;
mod errors;
mod extra_data;
mod inputs;
mod messages;
mod outputs;
mod state;
mod traits;

// Include generated SSZ types from build.rs output
#[allow(
    clippy::all,
    unreachable_pub,
    clippy::allow_attributes,
    clippy::absolute_paths,
    reason = "generated code"
)]
mod ssz_generated {
    include!(concat!(env!("OUT_DIR"), "/generated.rs"));
}

pub use commit::*;
pub use errors::{EnvError, EnvResult, MessageDecodeError, MessageDecodeResult};
pub use extra_data::*;
pub use inputs::ExecPayload;
pub use messages::*;
pub use outputs::ExecBlockOutput;
pub use ssz_generated::ssz::{self as ssz, state::*};
pub use state::PendingInputType;
pub use strata_identifiers::Hash;
pub use traits::{
    BlockAssembler, ExecBlock, ExecBlockBody, ExecHeader, ExecPartialState, ExecutionEnvironment,
};
