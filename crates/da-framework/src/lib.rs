mod codec;
pub use codec::{
    Codec, CodecError, CodecResult, Decoder, Encoder, LargeVec, MediumVec, SmallVec, decode_vec,
    encode_to_vec,
};

mod compound;
pub use compound::CompoundMember;

mod counter;
pub use counter::DaCounter;

mod errors;
pub use errors::{BuilderError, DaError};

mod register;
pub use register::DaRegister;

mod traits;
pub use traits::{ContextlessDaWrite, DaBuilder, DaWrite};

mod queue;
pub use queue::{DaQueue, DaQueueTarget};

mod linear_acc;
pub use linear_acc::{DaLinacc, LinearAccumulator};
