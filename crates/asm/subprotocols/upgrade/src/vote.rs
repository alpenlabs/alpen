/// An aggregated signature over a subset of signers in a MultisigConfig,
/// identified by their positions in the config’s key list.
pub struct AggregatedVote {
    voter_indices: Vec<u8>,
    agg_signature: u128, // FIXME:
}
