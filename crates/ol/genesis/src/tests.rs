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
    let expected_root: Buf32 = "d05e3d75c24f8924db89840867fb2bec4e1d99016f856fd98657b3d7ac84226d"
        .parse()
        .unwrap();
    let expected_block_id: OLBlockId =
        "f0845191cfb4b2c20915189e536187d15a38acfbccd68818bdbe1a53317744e2"
            .parse::<Buf32>()
            .unwrap()
            .into();
    assert_eq!(state_root, expected_root);
    assert_eq!(artifacts.commitment.blkid(), &expected_block_id);
}
