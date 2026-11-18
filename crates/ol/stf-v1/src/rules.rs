//! Rules that later specs amend in the V1 code.

/// Selects the rules that later specs amend in this crate's code.
///
/// Every spec from V1 on runs this code. The V1 and V2 rules are fixed: the
/// checkpoint programs that prove their epochs were published, so a node must
/// accept exactly what those programs accept. A rule that changes after them
/// is therefore an amendment that a later spec turns on, and the code keeps
/// the earlier rule for the specs that do not.
///
/// [`Self::NONE`] runs the rules as V1 and V2 fix them. The operations that
/// build their own context run those unless they take amendments. The
/// operations that take a context, such as
/// [`process_single_tx`](crate::process_single_tx), run the amendments it
/// carries, which are [`Self::NONE`] unless the caller set them with
/// [`BasicExecContext::with_amendments`](crate::BasicExecContext::with_amendments).
/// `strata-ol-stf` sets them for the spec it dispatches to.
///
/// Epoch replay from DA takes no amendments: it applies a diff and replays
/// manifests, and no amended rule runs there.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuleAmendments {
    canonical_withdrawal_len_prefix: bool,
    accumulator_proofs_bind_index: bool,
}

impl RuleAmendments {
    /// No amendments: the rules of V1 and V2.
    pub const NONE: Self = Self {
        canonical_withdrawal_len_prefix: false,
        accumulator_proofs_bind_index: false,
    };

    /// The amendments of the V3 rules.
    ///
    /// A withdrawal message to the bridge gateway must write its descriptor
    /// length in the shortest layout, and an accumulator proof must lead to
    /// the peak that holds the claimed index.
    pub const V3: Self = Self {
        canonical_withdrawal_len_prefix: true,
        accumulator_proofs_bind_index: true,
    };

    /// Returns whether a withdrawal message must write its descriptor length
    /// in the shortest layout.
    ///
    /// Without the amendment a withdrawal message is valid in every layout of
    /// the length prefix, as `strata-codec` decoded it before v0.5.1. A
    /// message that is not a valid withdrawal sends its value to limbo instead
    /// of emitting a withdrawal log, so the two rules reach different states.
    pub(crate) fn requires_canonical_withdrawal_len_prefix(&self) -> bool {
        self.canonical_withdrawal_len_prefix
    }

    /// Returns whether an accumulator proof must lead to the peak that holds
    /// the claimed index.
    ///
    /// Without the amendment the proof's length selects the peak and only the
    /// low bits of the index are checked against the path, as `strata-merkle`
    /// verified before v0.5.0. That also accepts an entry of one peak under an
    /// index in another peak with the same low bits.
    pub(crate) fn accumulator_proofs_bind_index(&self) -> bool {
        self.accumulator_proofs_bind_index
    }
}
