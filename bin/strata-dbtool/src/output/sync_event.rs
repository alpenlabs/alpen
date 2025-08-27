//! Sync event formatting implementations

use strata_primitives::prelude::L1BlockCommitment;

use super::helpers::porcelain_field;

/// Format L1 block commitment for porcelain output
fn format_l1_block_commitment(l1_ref: &L1BlockCommitment, prefix: &str) -> Vec<String> {
    let mut output = Vec::new();

    output.push(porcelain_field(
        &format!("{prefix}.height"),
        l1_ref.height(),
    ));
    output.push(porcelain_field(
        &format!("{prefix}.blkid"),
        format!("{:?}", l1_ref.blkid()),
    ));

    output
}
