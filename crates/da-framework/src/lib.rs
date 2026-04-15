mod codec;
mod compound;
mod counter;
mod errors;
mod linear_acc;
mod queue;
mod register;
mod traits;
mod varint64;

pub use codec::{
    Codec, CodecError, CodecResult, Decoder, Encoder, Varint, decode_buf_exact, decode_map,
    decode_map_with, decode_vec, decode_vec_with, encode_map, encode_map_with, encode_to_vec,
    encode_vec, encode_vec_with,
};
pub use compound::{BitSeqReader, BitSeqWriter, Bitmap, CompoundMember};
pub use counter::{CounterScheme, DaCounter, DaCounterBuilder, counter_schemes};
pub use errors::{BuilderError, DaError};
pub use linear_acc::{DaLinacc, LinearAccumulator};
pub use queue::{DaQueue, DaQueueBuilder, DaQueueTarget, QueueView};
pub use register::DaRegister;
pub use traits::{ContextlessDaWrite, DaBuilder, DaWrite};
pub use varint64::{SignedVarInt, UnsignedVarInt};

#[cfg(test)]
/// Returns a boundary-heavy [`u64`] strategy shared by DA framework proptests.
///
/// The fixed values are chosen to hit the codec thresholds we care about:
/// zero and one for degenerate cases; `63`/`64`, `127`/`128`, `255`/`256`,
/// `8191`/`8192`, and `16383`/`16384` for signed and unsigned varint byte-width
/// transitions; [`u32::MAX`] for a large but common boundary; and `u64::MAX - 1`
/// plus [`u64::MAX`] for full-range overflow and saturation paths. Arbitrary
/// [`u64`] values are still included so the strategy keeps broad coverage beyond
/// the hand-picked edges.
pub(crate) fn boundary_u64_strategy() -> impl proptest::strategy::Strategy<Value = u64> {
    use proptest::prelude::*;

    prop_oneof![
        Just(0),
        Just(1),
        Just(63),
        Just(64),
        Just(127),
        Just(128),
        Just(255),
        Just(256),
        Just(8191),
        Just(8192),
        Just(16383),
        Just(16384),
        Just(u32::MAX as u64),
        Just(u64::MAX - 1),
        Just(u64::MAX),
        any::<u64>(),
    ]
}
