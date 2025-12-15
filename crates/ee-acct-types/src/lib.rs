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

pub use commit::{CommitBlockData, CommitChainSegment};
pub use errors::{EnvError, EnvResult, MessageDecodeError, MessageDecodeResult};
pub use extra_data::UpdateExtraData;
pub use inputs::ExecPayload;
pub use messages::*;
pub use outputs::ExecBlockOutput;
pub use state::{EeAccountState, PendingFinclEntry, PendingInputEntry, PendingInputType};
pub use strata_identifiers::Hash;
pub use traits::{
    BlockAssembler, ExecBlock, ExecBlockBody, ExecHeader, ExecPartialState, ExecutionEnvironment,
};
