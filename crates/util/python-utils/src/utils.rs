use std::str::FromStr;

use bdk_wallet::{
    bitcoin::{bip32::Xpriv, taproot::LeafVersion, Address, TapNodeHash},
    miniscript::{Miniscript, Tap},
    template::DescriptorTemplateOut,
};
use pyo3::prelude::*;
use secp256k1::{Keypair, XOnlyPublicKey, SECP256K1};
use shrex::decode_alloc;
use strata_primitives::{
    bitcoin_bosd::Descriptor,
    constants::{RECOVER_DELAY, UNSPENDABLE_PUBLIC_KEY},
};

use crate::{
    constants::GENERAL_WALLET_KEY_PATH,
    error::Error,
    taproot::{musig_aggregate_pks_inner, ExtractP2trPubkey},
};

/// The descriptor for the bridge-in transaction.
///
/// # Note
///
/// The descriptor is a Tapscript that enforces the following conditions:
///
/// - The funds can be spent by the bridge operator.
/// - The funds can be spent by the recovery address after a delay.
///
/// # Returns
///
/// The descriptor and the script hash for the recovery path.
///
/// required for testing
#[allow(dead_code)]
pub(crate) fn bridge_in_descriptor(
    bridge_pubkey: XOnlyPublicKey,
    recovery_address: Address,
) -> Result<(DescriptorTemplateOut, TapNodeHash), Error> {
    let recovery_xonly_pubkey = recovery_address.extract_p2tr_pubkey()?;

    let desc = bdk_wallet::descriptor!(
        tr(UNSPENDABLE_PUBLIC_KEY, {
            pk(bridge_pubkey),
            and_v(v:pk(recovery_xonly_pubkey),older(RECOVER_DELAY))
        })
    )
    .expect("valid descriptor");

    // we have to do this to obtain the script hash
    // i have tried to extract it directly from the desc above
    // it is a massive pita
    let recovery_script = Miniscript::<XOnlyPublicKey, Tap>::from_str(&format!(
        "and_v(v:pk({recovery_xonly_pubkey}),older(1008))",
    ))
    .expect("valid recovery script")
    .encode();

    let recovery_script_hash = TapNodeHash::from_script(&recovery_script, LeafVersion::TapScript);

    Ok((desc, recovery_script_hash))
}

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

/// Parses operator secret keys from hex strings
pub(crate) fn parse_operator_keys(
    operator_keys: &[String],
) -> Result<(Vec<Keypair>, XOnlyPublicKey), Error> {
    let result: Vec<Keypair> = operator_keys
        .iter()
        .enumerate()
        .map(|(i, key)| {
            let xpriv = Xpriv::from_str(key)
                .map_err(|e| Error::BridgeBuilder(format!("Invalid operator key {}: {}", i, e)))
                .unwrap();

            let xp = xpriv
                .derive_priv(SECP256K1, &GENERAL_WALLET_KEY_PATH)
                .expect("good child key");

            let mut sk = xp.private_key;
            let pk = secp256k1::PublicKey::from_secret_key(SECP256K1, &sk);

            // This is very important because datatool and bridge does this way.
            // (x,P) and (x,-P) don't add to same group element, so in order to be consistent
            // we are only choosing even one so
            // if not even
            if pk.serialize()[0] != 0x02 {
                // Flip to even-Y equivalent
                sk = sk.negate();
            }

            Keypair::from_secret_key(SECP256K1, &sk)
        })
        .collect();

    let x_only_keys: Vec<XOnlyPublicKey> = result
        .iter()
        .map(|pair| XOnlyPublicKey::from_keypair(pair).0)
        .collect();

    Ok((result, musig_aggregate_pks_inner(x_only_keys)?))
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
