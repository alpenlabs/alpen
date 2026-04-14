//! L1 client setup and CLI error mapping.

use bitcoind_async_client::{traits::Reader, Auth, Client};
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::config::VerifierConfig;

const DISABLE_CLIENT_RETRIES: u8 = 0;

/// Builds a bitcoind client and verifies readiness before fetch starts.
pub(crate) async fn create_ready_client(config: &VerifierConfig) -> Result<Client, DisplayedError> {
    let client = Client::new(
        config.bitcoind_url.clone(),
        Auth::UserPass(
            config.bitcoind_rpc_user.clone(),
            config.bitcoind_rpc_password.clone(),
        ),
        Some(DISABLE_CLIENT_RETRIES),
        None,
        None,
    )
    .user_error("failed to initialize bitcoind client")?;

    Reader::get_blockchain_info(&client)
        .await
        .internal_error("bitcoind not ready for fetch")?;

    Ok(client)
}
