use bitcoin::params::{MAINNET, Params};
use rkyv::{
    Archived, Place, Resolver,
    rancor::Fallible,
    with::{ArchiveWith, DeserializeWith, SerializeWith},
};
use serde::{Deserialize, Serialize};

/// Serializer for [`Params`] as network index for rkyv.
#[derive(Debug, Clone, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct BtcParams(#[rkyv(with = ParamsAsNetwork)] Params);

/// Serializer for [`Params`] as network index for rkyv.
struct ParamsAsNetwork;

impl ArchiveWith<Params> for ParamsAsNetwork {
    type Archived = Archived<u8>;
    type Resolver = Resolver<u8>;

    fn resolve_with(field: &Params, resolver: Self::Resolver, out: Place<Self::Archived>) {
        rkyv::Archive::resolve(&network_to_tag(field.network), resolver, out);
    }
}

impl<S> SerializeWith<Params, S> for ParamsAsNetwork
where
    S: Fallible + ?Sized,
    u8: rkyv::Serialize<S>,
{
    fn serialize_with(field: &Params, serializer: &mut S) -> Result<Self::Resolver, S::Error> {
        rkyv::Serialize::serialize(&network_to_tag(field.network), serializer)
    }
}

impl<D> DeserializeWith<Archived<u8>, Params, D> for ParamsAsNetwork
where
    D: Fallible + ?Sized,
    Archived<u8>: rkyv::Deserialize<u8, D>,
{
    fn deserialize_with(field: &Archived<u8>, deserializer: &mut D) -> Result<Params, D::Error> {
        let tag = rkyv::Deserialize::deserialize(field, deserializer)?;
        let network = network_from_tag(tag);
        Ok(Params::from(network))
    }
}

fn network_to_tag(network: bitcoin::Network) -> u8 {
    match network {
        bitcoin::Network::Bitcoin => 0,
        bitcoin::Network::Testnet => 1,
        bitcoin::Network::Signet => 2,
        bitcoin::Network::Regtest => 3,
        _ => 255,
    }
}

fn network_from_tag(tag: u8) -> bitcoin::Network {
    match tag {
        0 => bitcoin::Network::Bitcoin,
        1 => bitcoin::Network::Testnet,
        2 => bitcoin::Network::Signet,
        3 => bitcoin::Network::Regtest,
        _ => panic!("invalid network tag {tag}"),
    }
}

impl PartialEq for BtcParams {
    fn eq(&self, other: &Self) -> bool {
        // Just compare the network since all other params derive from it
        self.0.network == other.0.network
    }
}

impl Eq for BtcParams {}

impl Default for BtcParams {
    fn default() -> Self {
        BtcParams(MAINNET.clone())
    }
}

impl Serialize for BtcParams {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // Just serialize the network - the rest can be derived from it
        self.0.network.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BtcParams {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let network = bitcoin::Network::deserialize(deserializer)?;
        Ok(BtcParams::from(Params::from(network)))
    }
}

impl<'a> arbitrary::Arbitrary<'a> for BtcParams {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let networks = [
            bitcoin::Network::Bitcoin,
            bitcoin::Network::Testnet,
            bitcoin::Network::Signet,
            bitcoin::Network::Regtest,
        ];
        let network = u.choose(&networks)?;
        Ok(BtcParams::from(Params::from(*network)))
    }
}

impl From<Params> for BtcParams {
    fn from(params: Params) -> Self {
        BtcParams(params)
    }
}

impl BtcParams {
    pub fn into_inner(self) -> Params {
        self.0
    }

    pub fn inner(&self) -> &Params {
        &self.0
    }

    pub fn difficulty_adjustment_interval(&self) -> u64 {
        self.0.difficulty_adjustment_interval()
    }
}

impl AsRef<Params> for BtcParams {
    fn as_ref(&self) -> &Params {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use bitcoin::Network;
    use rkyv::rancor::Error as RkyvError;

    use super::*;

    #[test]
    fn test_all_networks_serialization() {
        let networks = [
            Network::Bitcoin,
            Network::Testnet,
            Network::Signet,
            Network::Regtest,
        ];

        for network in networks {
            let params = BtcParams::from(Params::from(network));

            // Test rkyv
            let bytes = rkyv::to_bytes::<RkyvError>(&params).unwrap();
            let rkyv_result = rkyv::from_bytes::<BtcParams, RkyvError>(&bytes).unwrap();
            assert_eq!(params, rkyv_result);

            // Test Serde
            let json_data = serde_json::to_string(&params).unwrap();
            let serde_result: BtcParams = serde_json::from_str(&json_data).unwrap();
            assert_eq!(params, serde_result);
        }
    }
}
