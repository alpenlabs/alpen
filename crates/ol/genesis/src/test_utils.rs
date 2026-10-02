//! Genesis fixtures for networks launched on 0.3.0.
//!
//! The `MN0_SHAPED_V0_GENESIS_*` constants were computed on `releases/0.3.0`
//! (`4221af71`) by a scratch test, not committed: it parsed
//! [`MN0_SHAPED_V0_PARAMS`] in that release's flat params layout (the
//! `genesis` members and `bridge_params` at the top level, and no `spec`) and
//! ran that release's `build_genesis_artifacts` on them.

use strata_ol_params::OLParams;
use strata_ol_state_types::OLSpecId;

/// Params shaped like MN0's, converted to the nested layout with `"spec": 0`:
/// MN0's genesis header, account ID, inner state, L1 anchor and bridge params,
/// with an always-accept predicate in place of MN0's EE key.
pub const MN0_SHAPED_V0_PARAMS: &str = r#"{
    "genesis": {
        "spec": 0,
        "header": {
            "timestamp": 0,
            "slot": 0,
            "epoch": 0,
            "parent_blkid": "0000000000000000000000000000000000000000000000000000000000000000",
            "body_root": "0000000000000000000000000000000000000000000000000000000000000000",
            "logs_root": "0000000000000000000000000000000000000000000000000000000000000000"
        },
        "accounts": {
            "0101010101010101010101010101010101010101010101010101010101010101": {
                "predicate": "AlwaysAccept",
                "inner_state": "308ce726a90fd45d3638fd86dec816cca262edc0d5acee9b130cfa33dbb740b0",
                "balance": 0
            }
        },
        "last_l1_block": {
            "height": 961729,
            "blkid": "0000000000000000000055fbd96192d25981163ea0f18accc6d1e22cafe84b6f"
        }
    },
    "runtime": {
        "bridge_params": {
            "denomination": 200000000,
            "max_withdrawal_amount": null,
            "max_withdrawal_descriptor_len": 81
        }
    }
}"#;

/// Genesis state root `releases/0.3.0` computes for [`MN0_SHAPED_V0_PARAMS`].
pub const MN0_SHAPED_V0_GENESIS_STATE_ROOT: &str =
    "3591a7d6d96d00536bf523389d24b4c9c892ff8ea8476868dfd10eede851d313";

/// Genesis block ID `releases/0.3.0` computes for [`MN0_SHAPED_V0_PARAMS`].
pub const MN0_SHAPED_V0_GENESIS_BLKID: &str =
    "a4d059617b1d92175af23009ec462026606ffd9770567dd5d67e04df05aa2642";

/// Returns [`MN0_SHAPED_V0_PARAMS`] with `spec` as the genesis spec.
pub fn mn0_shaped_params(spec: OLSpecId) -> OLParams {
    let json = MN0_SHAPED_V0_PARAMS.replacen(
        r#""spec": 0"#,
        &format!(r#""spec": {}"#, u32::from(spec)),
        1,
    );
    serde_json::from_str(&json).expect("MN0-shaped params parse")
}
