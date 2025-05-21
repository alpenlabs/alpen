mod codec;
pub use codec::{
    Codec, CodecError, CodecResult, Decoder, Encoder, LargeVec, MediumVec, SmallVec, decode_vec,
    encode_to_vec,
};

mod compound;
pub use compound::CompoundMember;

mod counter;
pub use counter::DaCounter;

mod register;
pub use register::DaRegister;

mod traits;
pub use traits::DaWrite;

mod queue;
pub use queue::{DaQueue, DaQueueTarget};

mod linear_acc;
pub use linear_acc::{DaLinacc, LinearAccumulator};
