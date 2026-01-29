//! General-purpose snark account runtime library.

mod errors;
mod message;
mod private_input;
mod program_processing;
mod traits;

pub use errors::*;
pub use message::{InputMessage, MsgMeta};
pub use program_processing::*;
pub use traits::*;
