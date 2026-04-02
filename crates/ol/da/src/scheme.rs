//! DA scheme implementations for OL state.

use strata_da_framework::DaWrite;
use strata_ledger_types::{
    IAccountState, IAccountStateConstructible, ISnarkAccountStateConstructible, IStateAccessor,
};

use crate::{DaResult, DaScheme, OLDaPayloadV1, OLStateDiff};

/// DA scheme v1 for applying OL checkpoint state diffs to any state accumulator
/// implementing [`IStateAccessor`].
#[derive(Debug, Default)]
pub struct OLDaSchemeV1;

impl<S> DaScheme<S> for OLDaSchemeV1
where
    S: IStateAccessor,
    S::AccountState: IAccountStateConstructible,
    <S::AccountState as IAccountState>::SnarkAccountState: ISnarkAccountStateConstructible,
{
    type Diff = OLDaPayloadV1;

    /// Applies an [`OLDaPayloadV1`] to the state accumulator.
    ///
    /// Converts the payload's raw [`StateDiff`](crate::StateDiff) into a
    /// typed [`OLStateDiff`], then runs the two-phase DA write protocol:
    ///
    /// 1. poll_context() — validates diff entries against current state.
    /// 2. apply() — mutates the accumulator with the validated diff.
    fn apply_to_state(diff: Self::Diff, acc: &mut S) -> DaResult<()> {
        let state_diff = OLStateDiff::<S>::from(diff.state_diff);
        DaWrite::poll_context(&state_diff, acc, &())?;
        DaWrite::apply(&state_diff, acc, &())
    }
}

#[cfg(test)]
mod tests {
    use strata_da_framework::DaCounter;
    use strata_ledger_types::IStateAccessor;
    use strata_ol_stf::test_utils::create_test_genesis_state;

    use super::*;
    use crate::{GlobalStateDiff, LedgerDiff, StateDiff};

    #[test]
    fn test_ol_da_scheme_v1_apply_updates_global_slot() {
        let mut state = create_test_genesis_state();
        let start_slot = state.cur_slot();

        let diff = StateDiff::new(
            GlobalStateDiff::new(DaCounter::new_changed(1u16)),
            LedgerDiff::default(),
        );
        let payload = OLDaPayloadV1::new(diff);

        OLDaSchemeV1::apply_to_state(payload, &mut state).expect("apply scheme");

        assert_eq!(state.cur_slot(), start_slot + 1);
    }
}
