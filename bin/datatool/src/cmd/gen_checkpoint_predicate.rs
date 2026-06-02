//! `gen-checkpoint-predicate` subcommand: prints the checkpoint predicate.

use crate::{
    args::{CmdContext, SubcCheckpointPredicate},
    checkpoint_predicate::resolve_checkpoint_predicate,
};

/// Executes the `gen-checkpoint-predicate` subcommand.
///
/// Resolves the checkpoint predicate (the SP1 checkpoint verifying key under the
/// `sp1-builder` feature, or the feature-gated/overridden default) and prints it
/// to stdout in the same `"<Type>:<hex>"` form it takes inside the ASM params.
pub(super) fn exec(cmd: SubcCheckpointPredicate, _ctx: &mut CmdContext) -> anyhow::Result<()> {
    let predicate = resolve_checkpoint_predicate(cmd.checkpoint_predicate)?;
    let value = serde_json::to_value(&predicate)?;
    let serialized = value
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("checkpoint predicate did not serialize to a string"))?;
    println!("{serialized}");

    Ok(())
}
