use std::{fmt, str::FromStr};

/// Represents a type of network, either Alpen's signet or Alpen
#[non_exhaustive]
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum NetworkType {
    Signet,
    Alpen,
}

#[derive(Clone, Copy, Debug)]
pub struct InvalidNetwork;

impl FromStr for NetworkType {
    type Err = InvalidNetwork;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bitcoin" | "signet" => Ok(Self::Signet),
            "alpen" => Ok(Self::Alpen),
            _ => Err(InvalidNetwork),
        }
    }
}

impl fmt::Display for NetworkType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NetworkType::Signet => "signet",
            NetworkType::Alpen => "alpen",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bitcoin_network_name() {
        assert_eq!(
            NetworkType::from_str("bitcoin").unwrap(),
            NetworkType::Signet
        );
    }
}
