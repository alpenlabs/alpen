//! Bitcoin RPC client helpers shared across subcommands.

use std::path::PathBuf;

use bdk_bitcoind_rpc::bitcoincore_rpc::{Auth, Client};

use crate::error::Error;

/// Creates a new `bitcoind` RPC client.
pub(crate) fn new_bitcoind_client(
    url: &str,
    rpc_cookie: Option<&PathBuf>,
    rpc_user: Option<&str>,
    rpc_pass: Option<&str>,
) -> Result<Client, Error> {
    Client::new(
        url,
        match (rpc_cookie, rpc_user, rpc_pass) {
            (None, None, None) => Auth::None,
            (Some(path), _, _) => Auth::CookieFile(path.clone()),
            (_, Some(user), Some(pass)) => Auth::UserPass(user.into(), pass.into()),
            (_, Some(_), None) => panic!("rpc auth: missing rpc_pass"),
            (_, None, Some(_)) => panic!("rpc auth: missing rpc_user"),
        },
    )
    .map_err(|_| Error::RpcClient)
}
