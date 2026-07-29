//! Shared Alpen EE account presets used across CLI subcommands.
//!
//! A preset names one of the well-known Alpen EE accounts. Different
//! subcommands map a preset onto different resources: `deposit` resolves it to
//! an [`AccountSerial`], while `withdraw` resolves it to an RPC endpoint.

use std::str::FromStr;

use strata_identifiers::{AccountSerial, SYSTEM_RESERVED_ACCTS};

use crate::constants::{ALPN_EE_ACCT_SERIAL, NPAL_EE_ACCT_SERIAL};

/// Named Alpen EE account presets selectable on subcommands such as `deposit`
/// and `withdraw`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EePreset {
    /// The default Alpen EE account (serial [`ALPN_EE_ACCT_SERIAL`], RPC
    /// endpoint `alpen_endpoint`).
    Alpn,
    /// The secondary Alpen EE account (serial [`NPAL_EE_ACCT_SERIAL`], RPC
    /// endpoint `nepal_endpoint`).
    Npal,
}

impl EePreset {
    /// Returns the [`AccountSerial`] backing this preset.
    pub fn serial(self) -> AccountSerial {
        match self {
            EePreset::Alpn => ALPN_EE_ACCT_SERIAL,
            EePreset::Npal => NPAL_EE_ACCT_SERIAL,
        }
    }

    /// Returns the numeric account serial backing this preset.
    ///
    /// This mirrors [`serial`](Self::serial) as a plain `u32`, for callers that
    /// need to key on the serial (e.g. the `transfer` command's account-id
    /// lookup) rather than an [`AccountSerial`].
    pub fn serial_num(self) -> u32 {
        match self {
            EePreset::Alpn => SYSTEM_RESERVED_ACCTS,
            EePreset::Npal => SYSTEM_RESERVED_ACCTS + 1,
        }
    }
}

impl FromStr for EePreset {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_uppercase().as_str() {
            "ALPN" => Ok(EePreset::Alpn),
            "NPAL" => Ok(EePreset::Npal),
            other => Err(format!(
                "unknown EE preset '{other}', expected 'ALPN' or 'NPAL'"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ee_preset_from_str() {
        assert_eq!(EePreset::from_str("ALPN"), Ok(EePreset::Alpn));
        assert_eq!(EePreset::from_str("alpn"), Ok(EePreset::Alpn));
        assert_eq!(EePreset::from_str("NPAL"), Ok(EePreset::Npal));
        assert_eq!(EePreset::from_str("npal"), Ok(EePreset::Npal));
        assert!(EePreset::from_str("nope").is_err());
    }

    #[test]
    fn ee_preset_serial_mapping() {
        assert_eq!(EePreset::Alpn.serial(), ALPN_EE_ACCT_SERIAL);
        assert_eq!(EePreset::Npal.serial(), NPAL_EE_ACCT_SERIAL);
    }

    #[test]
    fn ee_preset_serial_num_matches_serial() {
        assert_eq!(EePreset::Alpn.serial(), AccountSerial::new(EePreset::Alpn.serial_num()));
        assert_eq!(EePreset::Npal.serial(), AccountSerial::new(EePreset::Npal.serial_num()));
        assert_eq!(EePreset::Npal.serial_num(), EePreset::Alpn.serial_num() + 1);
    }
}
