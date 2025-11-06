use std::error::Error;

use strata_ledger_types::LedgerInterface;

use crate::VerifiedUpdate;

/// Applies snark update outputs to the ledger.
pub fn apply_update_outputs<'a, E: Error, L: LedgerInterface<E>>(
    ledger_ref: &mut L,
    verified_update: VerifiedUpdate<'a>,
) -> Result<(), E> {
    let outputs = verified_update.operation().outputs();
    let transfers = outputs.transfers();
    let messages = outputs.messages();

    // Process transfers
    for transfer in transfers {
        ledger_ref.send_transfer(transfer.dest(), transfer.value())?;
    }

    // Process messages
    for msg in messages {
        let payload = msg.payload();
        ledger_ref.send_message(msg.dest(), payload.clone())?;
    }

    Ok(())
}
