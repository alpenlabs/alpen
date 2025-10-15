use bdk_wallet::bitcoin::XOnlyPublicKey;
use shrex::decode_alloc;
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::error::Error;

/// Converts a [`XOnlyPublicKey`] to a BOSD [`Descriptor`].
pub(crate) fn xonlypk_to_descriptor_inner(xonly: &str) -> Result<String, Error> {
    // convert the hex-string into bytes
    let xonly_bytes = decode_alloc(xonly).map_err(|_| Error::XOnlyPublicKey)?;
    // parse the xonly public key
    let xonly = XOnlyPublicKey::from_slice(&xonly_bytes).map_err(|_| Error::XOnlyPublicKey)?;

    let descriptor: Descriptor = xonly.into();
    Ok(descriptor.to_string())
}


