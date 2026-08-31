use std::ops::{Deref, DerefMut};

use alloy::{
    network::EthereumWallet,
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
        Identity, ProviderBuilder, RootProvider,
    },
    transports::{
        http::reqwest::{
            header::{HeaderMap, HeaderValue, AUTHORIZATION},
            Client, Url,
        },
        Authorization,
    },
};

use crate::{seed::Seed, settings::Settings};

// alloy moment 💀
type Provider = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider,
>;

#[derive(Debug)]
pub struct AlpenWallet(Provider);

impl DerefMut for AlpenWallet {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Deref for AlpenWallet {
    type Target = Provider;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub struct AlpenEndpointParseError;

impl AlpenWallet {
    pub fn new(seed: &Seed, settings: &Settings) -> Result<Self, AlpenEndpointParseError> {
        Self::from_endpoint(seed, &settings.alpen_endpoint, settings.alpen_rpc_auth())
    }

    fn from_endpoint(
        seed: &Seed,
        endpoint: &str,
        auth: Option<(&str, &str)>,
    ) -> Result<Self, AlpenEndpointParseError> {
        let endpoint: Url = endpoint.parse().map_err(|_| AlpenEndpointParseError)?;
        let mut headers = HeaderMap::new();
        if let Some((username, password)) = auth {
            let mut value =
                HeaderValue::from_str(&Authorization::basic(username, password).to_string())
                    .expect("Basic authentication should produce a valid HTTP header");
            value.set_sensitive(true);
            headers.insert(AUTHORIZATION, value);
        }
        let client = Client::builder()
            .default_headers(headers)
            .build()
            .map_err(|_| AlpenEndpointParseError)?;
        let wallet = seed.get_alpen_wallet();

        let provider = ProviderBuilder::new()
            .wallet(wallet)
            .connect_reqwest(client, endpoint);

        Ok(Self(provider))
    }
}

#[cfg(test)]
mod tests {
    use shrex::Hex;

    use super::*;
    use crate::constants::SEED_LEN;

    #[test]
    fn constructs_provider_with_optional_basic_auth() {
        let seed = Seed::from_file(Hex([0; SEED_LEN]));

        assert!(AlpenWallet::from_endpoint(&seed, "https://rpc.example.com", None).is_ok());
        assert!(AlpenWallet::from_endpoint(
            &seed,
            "https://rpc.example.com",
            Some(("alpen", "secret")),
        )
        .is_ok());
    }

    #[test]
    fn rejects_invalid_endpoint() {
        let seed = Seed::from_file(Hex([0; SEED_LEN]));

        assert!(AlpenWallet::from_endpoint(&seed, "not a URL", None).is_err());
    }
}
