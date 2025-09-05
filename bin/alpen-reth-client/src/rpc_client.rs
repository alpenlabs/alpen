use strata_common::ws_client::{ManagedWsClient, WsClientConfig};

pub(crate) fn rpc_client(url: &str) -> ManagedWsClient {
    ManagedWsClient::new_with_default_pool(WsClientConfig {
        url: url.to_string(),
    })
}
