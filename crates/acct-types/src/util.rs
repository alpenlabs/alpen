//! Various utilities.

use digest::{Digest, generic_array::GenericArray};
use sha2::Sha256;
use strata_codec::{Codec, CodecError, Encoder};

struct DigestEnc<D> {
    digest: D,
}

impl<D: Digest> Encoder for DigestEnc<D> {
    fn write_buf(&mut self, buf: &[u8]) -> Result<(), CodecError> {
        self.digest.update(buf);
        Ok(())
    }
}

/// Computes the hash of a codec-encodable type, without materializing the whole
/// encoding into a buffer.
fn compute_codec_digest<T: Codec, D: Digest>(
    v: &T,
) -> Result<GenericArray<u8, D::OutputSize>, CodecError> {
    let mut enc = DigestEnc { digest: D::new() };
    v.encode(&mut enc)?;
    Ok(enc.digest.finalize())
}

/// Computes the SHA-256 hash of a codec-encodable type, without materializing the whole
/// encoding into a buffer.
pub fn compute_codec_sha256<T: Codec>(v: &T) -> Result<[u8; 32], CodecError> {
    let output = compute_codec_digest::<T, Sha256>(v)?;
    Ok(output.into())
}
