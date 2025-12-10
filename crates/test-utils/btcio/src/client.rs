use std::env;

use bitcoind_async_client::Client;
use corepc_node::Node;

/// Get the authentication credentials for a given `bitcoind` instance.
fn get_auth(bitcoind: &Node) -> (String, String) {
    let params = &bitcoind.params;
    let cookie_values = params.get_cookie_values().unwrap().unwrap();
    (cookie_values.user, cookie_values.password)
}

/// Create a new bitcoind node and RPC client for testing.
///
/// # Safety
/// This function sets the `BITCOIN_XPRIV_RETRIEVABLE` environment variable to enable
/// private key retrieval. This should only be used in test environments.
pub fn get_bitcoind_and_client() -> (Node, Client) {
    // setting the ENV variable `BITCOIN_XPRIV_RETRIEVABLE` to retrieve the xpriv
    // SAFETY: This is a test environment and we control the execution flow.
    unsafe {
        env::set_var("BITCOIN_XPRIV_RETRIEVABLE", "true");
    }
    let bitcoind = Node::new("bitcoind").unwrap();
    let url = bitcoind.rpc_url();
    let (user, password) = get_auth(&bitcoind);
    let client = Client::new(url, user, password, None, None).unwrap();
    (bitcoind, client)
}
