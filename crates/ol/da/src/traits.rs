//! DA traits.

use strata_codec::Codec;
use strata_ledger_types::IStateAccessor;

use crate::errors::DaResult;

/// Defines a DA scheme for some state accessor.
pub trait DaScheme<S: IStateAccessor> {
    /// The encoded diff structure.
    type Diff: Codec;

    /// Applies a diff to the state.
    fn apply_to_state(diff: Self::Diff, acc: &mut S) -> DaResult<()>;
}
