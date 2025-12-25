use corepc_node::BitcoinD;

/// Get the authentication credentials for a given `bitcoind` instance.
pub fn get_auth(bitcoind: &BitcoinD) -> (String, String) {
    let params = &bitcoind.params;
    let cookie_values = params.get_cookie_values().unwrap().unwrap();
    (cookie_values.user, cookie_values.password)
}
