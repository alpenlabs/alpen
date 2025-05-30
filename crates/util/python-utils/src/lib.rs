//! Python utilities for the Alpen codebase.

use pyo3::prelude::*;

mod constants;
mod drt;
mod error;
mod parse;
mod schnorr;
mod taproot;
mod utils;

use drt::{
    deposit_request_transaction, get_balance, get_balance_recovery, get_recovery_address,
    take_back_transaction,
};
use schnorr::{sign_schnorr_sig, verify_schnorr_sig};
use taproot::{
    convert_to_xonly_pk, drain_wallet, extract_p2tr_pubkey, get_address, get_change_address,
    musig_aggregate_pks, unspendable_address,
};
use utils::{
    address_to_descriptor, is_valid_bosd, opreturn_to_string, string_to_opreturn_descriptor,
    xonlypk_to_descriptor,
};

/// A Python module implemented in Rust. The name of this function must match
/// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
/// import the module.
#[pymodule]
fn strata_utils(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(deposit_request_transaction, m)?)?;
    m.add_function(wrap_pyfunction!(get_address, m)?)?;
    m.add_function(wrap_pyfunction!(get_change_address, m)?)?;
    m.add_function(wrap_pyfunction!(musig_aggregate_pks, m)?)?;
    m.add_function(wrap_pyfunction!(extract_p2tr_pubkey, m)?)?;
    m.add_function(wrap_pyfunction!(unspendable_address, m)?)?;
    m.add_function(wrap_pyfunction!(drain_wallet, m)?)?;
    m.add_function(wrap_pyfunction!(convert_to_xonly_pk, m)?)?;
    m.add_function(wrap_pyfunction!(take_back_transaction, m)?)?;
    m.add_function(wrap_pyfunction!(get_recovery_address, m)?)?;
    m.add_function(wrap_pyfunction!(get_balance, m)?)?;
    m.add_function(wrap_pyfunction!(get_balance_recovery, m)?)?;
    m.add_function(wrap_pyfunction!(sign_schnorr_sig, m)?)?;
    m.add_function(wrap_pyfunction!(verify_schnorr_sig, m)?)?;
    m.add_function(wrap_pyfunction!(address_to_descriptor, m)?)?;
    m.add_function(wrap_pyfunction!(xonlypk_to_descriptor, m)?)?;
    m.add_function(wrap_pyfunction!(string_to_opreturn_descriptor, m)?)?;
    m.add_function(wrap_pyfunction!(opreturn_to_string, m)?)?;
    m.add_function(wrap_pyfunction!(is_valid_bosd, m)?)?;

    Ok(())
}
