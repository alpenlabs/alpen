use arbitrary::Arbitrary;

use crate::deposit_request::DrtHeaderAux;

/// Information extracted from a deposit request transaction.
#[derive(Debug, Clone, Arbitrary)]
pub struct DrtInfo {
    /// SPS-50 auxiliary data from the transaction tag.
    header_aux: DrtHeaderAux,
}
