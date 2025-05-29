use std::ops::{Deref, DerefMut};

use alloy::{
    network::{Ethereum, EthereumWallet},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            WalletFiller,
        },
        Identity, ProviderBuilder, RootProvider,
    },
    transports::http::{Client, Http},
};

use crate::seed::Seed;

// alloy moment 💀
type Provider = FillProvider<
    JoinFill<
        JoinFill<
            Identity,
            JoinFill<GasFiller, JoinFill<BlobGasFiller, JoinFill<NonceFiller, ChainIdFiller>>>,
        >,
        WalletFiller<EthereumWallet>,
    >,
    RootProvider<Http<Client>>,
    Http<Client>,
    Ethereum,
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
    pub fn new(seed: &Seed, alpen_http_endpoint: &str) -> Result<Self, AlpenEndpointParseError> {
        let wallet = seed.get_alpen_wallet();

        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_http(
                alpen_http_endpoint
                    .parse()
                    .map_err(|_| AlpenEndpointParseError)?,
            );

        Ok(Self(provider))
    }
}
