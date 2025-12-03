//! Proof generation handler system

pub(crate) mod host;
pub(crate) mod remote;
pub(crate) mod traits;

pub use host::{HostInstance, HostResolver};
pub use remote::RemoteProofHandler;
pub use traits::{BoxedInput, InputFetcher, ProofHandler, ProofStorer};
