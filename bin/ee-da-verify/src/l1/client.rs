//! Initializes the bitcoind RPC client and verifies readiness.

use bitcoind_async_client::{traits::Reader, Auth, Client};
use strata_cli_common::errors::{DisplayableError, DisplayedError};

use crate::config::VerifierConfig;

/// Builds a bitcoind client and verifies readiness before fetch starts.
pub(crate) async fn create_ready_client(config: &VerifierConfig) -> Result<Client, DisplayedError> {
    let client = Client::new(
        config.bitcoind_url.clone(),
        Auth::UserPass(
            config.bitcoind_rpc_user.clone(),
            config.bitcoind_rpc_password.clone(),
        ),
        None,
        None,
        None,
    )
    .user_error("failed to initialize bitcoind client")?;

    Reader::get_blockchain_info(&client)
        .await
        .internal_error("bitcoind not ready for fetch")?;

    Ok(client)
}
