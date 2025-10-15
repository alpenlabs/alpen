//! Python utilities for the Alpen codebase.

use pyo3::prelude::*;

pub mod bridge;
mod constants;
mod error;
mod parse;
mod schnorr;
mod taproot;
mod utils;

use schnorr::sign_schnorr_sig;
use taproot::{convert_to_xonly_pk, extract_p2tr_pubkey, get_address, musig_aggregate_pks};
use utils::xonlypk_to_descriptor;

use crate::bridge::{dt::create_deposit_transaction, withdrawal::create_withdrawal_fulfillment};

/// A Python module implemented in Rust. The name of this function must match
/// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
/// import the module.
#[pymodule]
fn strata_utils(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(create_deposit_transaction, m)?)?;
    m.add_function(wrap_pyfunction!(create_withdrawal_fulfillment, m)?)?;
    m.add_function(wrap_pyfunction!(get_address, m)?)?;
    m.add_function(wrap_pyfunction!(musig_aggregate_pks, m)?)?;
    m.add_function(wrap_pyfunction!(extract_p2tr_pubkey, m)?)?;
    m.add_function(wrap_pyfunction!(convert_to_xonly_pk, m)?)?;
    m.add_function(wrap_pyfunction!(sign_schnorr_sig, m)?)?;
    m.add_function(wrap_pyfunction!(xonlypk_to_descriptor, m)?)?;

    Ok(())
}
