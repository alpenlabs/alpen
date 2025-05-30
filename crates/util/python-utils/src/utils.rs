use bdk_wallet::bitcoin::{Address, XOnlyPublicKey};
use pyo3::prelude::*;
use shrex::decode_alloc;
use strata_primitives::bitcoin_bosd::Descriptor;

use crate::error::Error;

/// Validates if a given string is a valid BOSD.
#[pyfunction]
pub(crate) fn is_valid_bosd(s: &str) -> bool {
    let result = s.parse::<Descriptor>();
    result.is_ok()
}

/// Converts an [`Address`] to a BOSD [`Descriptor`].
#[pyfunction]
pub(crate) fn address_to_descriptor(address: &str) -> Result<String, Error> {
    // parse the address
    let address = address
        .parse::<Address<_>>()
        .map_err(|_| Error::BitcoinAddress)?
        .assume_checked();

    let descriptor: Descriptor = address.into();
    Ok(descriptor.to_string())
}

/// Converts a [`XOnlyPublicKey`] to a BOSD [`Descriptor`].
#[pyfunction]
pub(crate) fn xonlypk_to_descriptor(xonly: &str) -> Result<String, Error> {
    // convert the hex-string into bytes
    let xonly_bytes = decode_alloc(xonly).map_err(|_| Error::XOnlyPublicKey)?;
    // parse the xonly public key
    let xonly = XOnlyPublicKey::from_slice(&xonly_bytes).map_err(|_| Error::XOnlyPublicKey)?;

    let descriptor: Descriptor = xonly.into();
    Ok(descriptor.to_string())
}

/// Converts a string to an `OP_RETURN` BOSD [`Descriptor`].
#[pyfunction]
pub(crate) fn string_to_opreturn_descriptor(s: &str) -> Result<String, Error> {
    let payload = s.as_bytes().to_vec();
    let descriptor = Descriptor::new_op_return(&payload).map_err(|_| Error::OpReturnTooLong)?;
    Ok(descriptor.to_string())
}

/// Converts an `OP_RETURN` scriptPubKey to a string.
#[pyfunction]
pub(crate) fn opreturn_to_string(s: &str) -> Result<String, Error> {
    // Remove the first 4 chars since we want the data
    // OP_RETURN <LEN> <DATA>
    let data = s.chars().skip(4).collect::<String>();

    // Now we need to decode the hex string
    let data_bytes = decode_alloc(&data).expect("could not decode hex");

    let string = String::from_utf8(data_bytes).expect("could not convert to string");
    Ok(string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_to_opreturn_descriptor_conversion() {
        let s = "hello world";
        // without the <OP_RETURN> <LEN> part and adding 00
        let expected = "0068656c6c6f20776f726c64";

        let result = string_to_opreturn_descriptor(s);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn opreturn_to_string_conversion() {
        // "hello world" taken from tx
        // 6dfb16dd580698242bcfd8e433d557ed8c642272a368894de27292a8844a4e75
        let s = "6a0b68656c6c6f20776f726c64";
        let expected = "hello world";

        let result = opreturn_to_string(s);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected);
    }
}
