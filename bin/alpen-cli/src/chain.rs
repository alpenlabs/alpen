use std::{fmt, str::FromStr};

/// Selects which chain the command operates on.
#[non_exhaustive]
#[derive(PartialEq, Eq, Debug, Clone)]
pub enum Chain {
    Bitcoin,
    Alpen,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidChain;

impl FromStr for Chain {
    type Err = InvalidChain;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bitcoin" | "l1" => Ok(Self::Bitcoin),
            "alpen" => Ok(Self::Alpen),
            _ => Err(InvalidChain),
        }
    }
}

impl fmt::Display for Chain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Chain::Bitcoin => "bitcoin",
            Chain::Alpen => "alpen",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_supported_chains() {
        assert_eq!("bitcoin".parse(), Ok(Chain::Bitcoin));
        assert_eq!("l1".parse(), Ok(Chain::Bitcoin));
        assert_eq!("alpen".parse(), Ok(Chain::Alpen));
        assert!("signet".parse::<Chain>().is_err());
    }
}
