use std::{collections::HashMap, str::FromStr, time::Duration};

use alloy::{
    network::TransactionBuilder,
    primitives::{Address as AlpenAddress, U256},
    providers::{Provider, WalletProvider},
    rpc::types::TransactionInput,
};
use alpen_reth_primitives::SubjectTransferCalldata;
use argh::FromArgs;
use indicatif::ProgressBar;
use strata_acct_types::AccountId;
use strata_cli_common::errors::{DisplayableError, DisplayedError};
use strata_identifiers::SubjectIdBytes;

use crate::{
    alpen::AlpenWallet,
    cmd::withdraw::{resolve_endpoint, resolve_explorer},
    constants::{SATS_TO_WEI, SUBJECT_TRANSFER_PRECOMPILE_ADDRESS},
    ee::EePreset,
    link::{OnchainObject, PrettyPrint},
    seed::Seed,
    settings::Settings,
};

/// Transfers value (and optional data) from your subject in one Alpen EE to a
/// subject in another EE, via the inter-EE subject-transfer precompile.
#[derive(FromArgs, PartialEq, Debug)]
#[argh(subcommand, name = "transfer")]
pub struct TransferArgs {
    /// source Alpen EE preset the transfer is sent from: "ALPN" (uses
    /// alpen_endpoint) or "NPAL" (uses nepal_endpoint). defaults to "ALPN".
    #[argh(positional)]
    source: Option<EePreset>,

    /// destination Alpen EE preset: "ALPN" or "NPAL". mutually exclusive with
    /// --serial. one of --to or --serial is required.
    #[argh(option)]
    to: Option<EePreset>,

    /// explicit destination account serial. mutually exclusive with --to.
    #[argh(option)]
    serial: Option<u32>,

    /// recipient EVM address in the destination EE. defaults to your own
    /// wallet address (a self-transfer into the other EE).
    #[argh(option)]
    recipient: Option<String>,

    /// amount to transfer in sats. any whole number; 0 is allowed for a
    /// data-only message.
    #[argh(option)]
    amount: u64,

    /// optional opaque hex payload delivered with the transfer.
    #[argh(option)]
    data: Option<String>,
}

/// Resolves the destination account serial from the `--to` preset or the
/// `--serial` flag. The two are mutually exclusive and one is required.
fn resolve_dest_serial(to: Option<EePreset>, serial: Option<u32>) -> Result<u32, DisplayedError> {
    match (to, serial) {
        (Some(_), Some(_)) => Err(DisplayedError::UserError(
            "--to and --serial are mutually exclusive; specify only one".to_string(),
            Box::new(()),
        )),
        (Some(preset), None) => Ok(preset.serial_num()),
        (None, Some(serial)) => Ok(serial),
        (None, None) => Err(DisplayedError::UserError(
            "specify a destination with --to <ALPN|NPAL> or --serial <N>".to_string(),
            Box::new(()),
        )),
    }
}

/// Looks up the destination [`AccountId`] the precompile requires from the
/// configured account-id table, erroring if the serial has no entry.
fn lookup_dest_account(
    serial: u32,
    ee_account_ids: &HashMap<u32, AccountId>,
) -> Result<AccountId, DisplayedError> {
    ee_account_ids.get(&serial).cloned().ok_or_else(|| {
        DisplayedError::UserError(
            format!(
                "no account id configured for serial {serial}; \
                 add it under [ee_account_ids] in the config file"
            ),
            Box::new(()),
        )
    })
}

pub async fn transfer(
    args: TransferArgs,
    seed: Seed,
    settings: Settings,
) -> Result<(), DisplayedError> {
    let endpoint = resolve_endpoint(
        args.source,
        &settings.alpen_endpoint,
        settings.nepal_endpoint.as_deref(),
    )?;
    let l2w = AlpenWallet::new(&seed, endpoint)
        .user_error("Invalid Alpen endpoint URL. Check the configuration")?;

    let dest_serial = resolve_dest_serial(args.to, args.serial)?;
    let dest_account = lookup_dest_account(dest_serial, &settings.ee_account_ids)?;

    // The destination subject is the recipient's address inside the destination
    // EE; default to our own address for a self-transfer.
    let recipient = match args.recipient {
        Some(a) => AlpenAddress::from_str(&a).user_error(format!(
            "Invalid recipient address '{a}'. Must be an EVM-compatible address"
        ))?,
        None => l2w.default_signer_address(),
    };
    let dest_subject = SubjectIdBytes::try_new(recipient.to_vec())
        .expect("an EVM address always fits in a 32-byte subject id")
        .to_subject_id();

    let data = match args.data {
        Some(hex) => shrex::decode_alloc(&hex).user_error("Invalid --data; must be hex")?,
        None => Vec::new(),
    };

    let calldata = SubjectTransferCalldata {
        dest_account,
        dest_subject,
        data,
    }
    .encode();

    let value = U256::from(args.amount as u128 * SATS_TO_WEI);
    println!(
        "Transferring {} sats to subject {recipient} on EE account serial {dest_serial}",
        args.amount,
    );

    let precompile = AlpenAddress::from_str(SUBJECT_TRANSFER_PRECOMPILE_ADDRESS)
        .expect("valid subject-transfer precompile address");
    let tx = l2w
        .transaction_request()
        .with_to(precompile)
        .with_value(value)
        .input(TransactionInput::new(calldata.into()));

    let pb = ProgressBar::new_spinner().with_message("Broadcasting transaction");
    pb.enable_steady_tick(Duration::from_millis(100));
    let res = l2w
        .send_transaction(tx)
        .await
        .internal_error("Failed to broadcast Alpen transaction")?;
    pb.finish_with_message("Broadcast successful");
    println!(
        "{}",
        OnchainObject::from(res.tx_hash())
            .with_maybe_explorer(resolve_explorer(
                args.source,
                settings.blockscout_endpoint.as_deref(),
                settings.blockscout_endpoint_nepal.as_deref(),
            ))
            .pretty(),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> HashMap<u32, AccountId> {
        let mut m = HashMap::new();
        m.insert(128, AccountId::new([0x11; 32]));
        m.insert(129, AccountId::new([0x22; 32]));
        m
    }

    #[test]
    fn dest_serial_from_preset() {
        assert_eq!(
            resolve_dest_serial(Some(EePreset::Npal), None).unwrap(),
            EePreset::Npal.serial_num()
        );
    }

    #[test]
    fn dest_serial_from_explicit() {
        assert_eq!(resolve_dest_serial(None, Some(200)).unwrap(), 200);
    }

    #[test]
    fn dest_serial_requires_one() {
        assert!(resolve_dest_serial(None, None).is_err());
    }

    #[test]
    fn dest_serial_rejects_both() {
        assert!(resolve_dest_serial(Some(EePreset::Alpn), Some(200)).is_err());
    }

    #[test]
    fn lookup_dest_account_found() {
        assert_eq!(
            lookup_dest_account(129, &ids()).unwrap(),
            AccountId::new([0x22; 32])
        );
        assert_eq!(
            lookup_dest_account(128, &ids()).unwrap(),
            AccountId::new([0x11; 32])
        );
    }

    #[test]
    fn lookup_dest_account_missing_errors() {
        assert!(lookup_dest_account(999, &ids()).is_err());
    }
}
