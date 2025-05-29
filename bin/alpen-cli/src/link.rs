use std::fmt::Display;

use alloy::primitives::{Address as AlpenAddress, TxHash};
use bdk_wallet::bitcoin;

/// Represents something that can be represented on-chain.
pub(crate) enum OnchainObject<'a> {
    Transaction(Txid<'a>),
    Address(Address<'a>),
}

impl<'a> From<&'a bitcoin::Address> for OnchainObject<'a> {
    fn from(address: &'a bitcoin::Address) -> Self {
        OnchainObject::Address(Address::Bitcoin(address))
    }
}

impl<'a> From<&'a AlpenAddress> for OnchainObject<'a> {
    fn from(address: &'a AlpenAddress) -> Self {
        OnchainObject::Address(Address::Alpen(address))
    }
}

impl<'a> From<&'a bitcoin::Txid> for OnchainObject<'a> {
    fn from(txid: &'a bitcoin::Txid) -> Self {
        OnchainObject::Transaction(Txid::Bitcoin(txid))
    }
}

impl<'a> From<&'a TxHash> for OnchainObject<'a> {
    fn from(txid: &'a TxHash) -> Self {
        OnchainObject::Transaction(Txid::Alpen(txid))
    }
}

impl<'a> OnchainObject<'a> {
    /// Creates a link to the object on the given explorer.
    /// Should be of the form `http{s}://{domain}`.
    ///
    /// Example: `https://mempool.space`
    pub(crate) const fn with_explorer<'b>(self, explorer: &'b str) -> Link<'a, 'b> {
        Link {
            object: self,
            explorer_ep: explorer,
        }
    }

    /// Creates a link to the object on the given explorer, if it exists.
    /// Should be of the form `http{s}://{domain}`.
    ///
    /// If `explorer` is `None`, the object will be represented as-is.
    ///
    /// This is primarily a helper for displaying an [`OnchainObject`] in a user-facing context.
    pub(crate) const fn with_maybe_explorer<'b>(
        self,
        explorer: Option<&'b str>,
    ) -> MaybeLink<'a, 'b> {
        match explorer {
            Some(dmn) => MaybeLink::Link(self.with_explorer(dmn)),
            None => MaybeLink::Object(self),
        }
    }
}

/// A helper trait for pretty printing something into a human-readable string.
/// Differs from [`Display`] in that it might add other wording outside of purely
/// the string representation of `self`.
///
/// E.g. a transaction might be prefixed with "Transaction: " or "Tx: ".
pub(crate) trait PrettyPrint {
    fn pretty(&self) -> String;
}

/// A helper enum for pretty printing something that might be a link or an object.
pub(crate) enum MaybeLink<'a, 'b> {
    /// A link to some explorer web page representing the object.
    Link(Link<'a, 'b>),
    /// Only an onchain object.
    Object(OnchainObject<'a>),
}

impl PrettyPrint for MaybeLink<'_, '_> {
    fn pretty(&self) -> String {
        match self {
            MaybeLink::Link(l) => l.pretty(),
            MaybeLink::Object(o) => o.pretty(),
        }
    }
}

impl PrettyPrint for OnchainObject<'_> {
    fn pretty(&self) -> String {
        match self {
            OnchainObject::Transaction(txid) => format!("Transaction ID: {txid}"),
            OnchainObject::Address(address) => format!("Address: {address}"),
        }
    }
}

impl Display for OnchainObject<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OnchainObject::Transaction(txid) => txid.fmt(f),
            OnchainObject::Address(address) => address.fmt(f),
        }
    }
}

/// A wrapper around a bitcoin transaction ID or a Alpen transaction ID.
#[derive(Clone, Copy)]
pub(crate) enum Txid<'a> {
    /// A transaction ID for a Bitcoin transaction.
    Bitcoin(&'a bitcoin::Txid),
    /// A transaction ID for a Alpen transaction.
    Alpen(&'a TxHash),
}

impl<'a> From<&'a TxHash> for Txid<'a> {
    fn from(v: &'a TxHash) -> Self {
        Self::Alpen(v)
    }
}

impl<'a> From<&'a bitcoin::Txid> for Txid<'a> {
    fn from(v: &'a bitcoin::Txid) -> Self {
        Self::Bitcoin(v)
    }
}

impl Display for Txid<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Txid::Bitcoin(txid) => txid.fmt(f),
            Txid::Alpen(txid) => txid.fmt(f),
        }
    }
}

/// A wrapper address that wraps either a bitcoin address or a alpen address.
#[derive(Clone, Copy)]
pub(crate) enum Address<'a> {
    Bitcoin(&'a bitcoin::Address),
    Alpen(&'a AlpenAddress),
}

impl Display for Address<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Address::Bitcoin(address) => address.fmt(f),
            Address::Alpen(address) => address.fmt(f),
        }
    }
}

/// A link to an object on an explorer.
///
/// This is primarily a helper for displaying an [`OnchainObject`] in a
/// user-facing context when a explorer URL is known.
pub(crate) struct Link<'a, 'b> {
    /// Object of the link (Transaction or Address)
    object: OnchainObject<'a>,
    /// Endpoint of the explorer (will be used to build the URL)
    explorer_ep: &'b str,
}

impl Display for Link<'_, '_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.object {
            OnchainObject::Transaction(ref txid) => match txid {
                Txid::Bitcoin(txid) => write!(f, "{}/tx/{}", self.explorer_ep, txid),
                Txid::Alpen(txid) => write!(f, "{}/tx/{}", self.explorer_ep, txid),
            },
            OnchainObject::Address(ref address) => match address {
                Address::Bitcoin(address) => {
                    write!(f, "{}/address/{}", self.explorer_ep, address)
                }
                Address::Alpen(address) => {
                    write!(f, "{}/address/{}", self.explorer_ep, address)
                }
            },
        }
    }
}

impl PrettyPrint for Link<'_, '_> {
    fn pretty(&self) -> String {
        match self.object {
            OnchainObject::Transaction(_) => format!("View transaction at {self}"),
            OnchainObject::Address(_) => format!("View address at {self}"),
        }
    }
}
