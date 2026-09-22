use strata_identifiers::{Buf32, OLBlockId};
use strata_ol_state_types::{IStateAccessor, OLSpecId};

use super::*;

#[test]
fn test_stable_container_genesis_commitments() {
    let params: OLParams =
        serde_json::from_str(include_str!("../tests/fixtures/genesis_params.json")).unwrap();
    let artifacts = build_genesis_artifacts(&params).unwrap();
    let state = MemoryStateBaseLayer::new(artifacts.ol_state);
    let state_root = state.compute_state_root().unwrap();
    assert_eq!(state.active_version(), OLSpecId::V1);
    assert_eq!(state.expected_version(), OLSpecId::V1);
    assert_eq!(&state_root, artifacts.ol_block.header().state_root());
    // Changing either commitment requires an intentional genesis reset.
    let expected_root: Buf32 = "d811a8bbf0a36aacd6451792ec664f574ae89660f415371dcc1110622d6a8a49"
        .parse()
        .unwrap();
    let expected_block_id: OLBlockId =
        "e159b3f0c24905d5e7a65bc0ebe15a57d3eafc45df8b4943a6dfcd4611dec6e2"
            .parse::<Buf32>()
            .unwrap()
            .into();
    assert_eq!(state_root, expected_root);
    assert_eq!(artifacts.commitment.blkid(), &expected_block_id);
}
