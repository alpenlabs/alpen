// why do we even have this module?

use bitcoin::params::{Params, MAINNET};

#[derive(Debug, Clone)]
pub struct BtcParams(Params);

impl From<Params> for BtcParams {
    fn from(params: Params) -> Self {
        BtcParams(params)
    }
}

impl BtcParams {
    pub fn into_inner(self) -> Params {
        self.0
    }

    pub fn inner(&self) -> &Params {
        &self.0
    }
}

/// Retrieves the [`bitcoin::params::Params`] for the Bitcoin network being used.
///
/// This method avoids creating a new struct because [`bitcoin::params::Params`] is marked as
/// `non_exhaustive`.
///
/// # Note
///
/// If adjustments to the parameters are required, modify them as shown below:
///
/// ```
/// use bitcoin::params::MAINNET;
/// use strata_primitives::l1::BtcParams;
///
/// fn get_btc_params() -> BtcParams {
///     let mut btc_params = MAINNET.clone();
///     btc_params.pow_target_spacing = 25 * 30; // Adjusted to 2.5 minutes
///     BtcParams::from(btc_params) // Return the modified `btc_params`
/// }
/// ```
///
/// # Returns
///
/// Returns the default Bitcoin Parameters used in our rollup.
pub fn get_btc_params() -> BtcParams {
    BtcParams(MAINNET.clone())
}
