#![expect(
    unused,
    reason = "will be used shortly, clippy is confused because of macros"
)]

mod codec;
pub mod compound;
mod counter;
mod errors;
mod linear_acc;
mod queue;
mod register;
mod traits;

pub use codec::{
    Codec, CodecError, CodecResult, Decoder, Encoder, decode_buf_exact, encode_to_vec,
};
pub use compound::CompoundMember;
pub use counter::{CounterScheme, DaCounter, DaCounterBuilder, counter_schemes};
pub use errors::{BuilderError, DaError};
pub use linear_acc::{DaLinacc, LinearAccumulator};
pub use queue::{DaQueue, DaQueueBuilder, DaQueueTarget, QueueView};
pub use register::DaRegister;
pub use traits::{ContextlessDaWrite, DaBuilder, DaWrite};
