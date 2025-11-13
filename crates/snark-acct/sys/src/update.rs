use strata_snark_acct_types::LedgerInterface;

use crate::VerifiedUpdate;

/// Applies verified snark account update outputs via the ledger interface.
///
/// Processes all transfers and messages in the update, delegating actual send operations
/// to the provided [`LedgerInterface`] implementation. This keeps snark-acct-sys independent
/// of STF implementation details.
///
/// Called after verification succeeds and before updating the snark account's proof state.
pub fn apply_update_outputs<'a, L: LedgerInterface>(
    ledger_impl: &mut L,
    verified_update: VerifiedUpdate<'a>,
) -> Result<(), L::Error> {
    let outputs = verified_update.operation().outputs();
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Process transfers
    for transfer in transfers {
        ledger_impl.send_transfer(transfer.dest(), transfer.value())?;
    }

    // Process messages
    for msg in messages {
        let payload = msg.payload();
        ledger_impl.send_message(msg.dest(), payload.clone())?;
    }

    Ok(())
}
