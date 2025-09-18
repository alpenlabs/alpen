use alpen_ee_primitives::{AccountAddress, BitcoinAddress, BitcoinAmount, EEAddress};

/// Message from OL to EE, read from this account's queue
#[derive(Debug, Clone)]
pub struct InboundMsgEnvelope {
    /// index of msg in account's input queue on OL
    pub idx: u64,
    /// Account where this message originated
    /// Equals bridge system account in deposits
    pub from: AccountAddress,
    /// Value transferred to this ee in this message
    pub value: BitcoinAmount,
    /// Message type and data
    pub msg: InboundMsg,
}

#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum InboundMsg {
    /// A deposit from bitcoin to EE.
    Deposit {
        /// address inside EE where the deposit should go to
        to: EEAddress,
        /*
        /// Any additional metadata attached to this deposit
        metadata: Option<Vec<u8>>,
        */
    },
    /*
    /// Native token (BTC) transfer from another account
    Transfer {
        destination: EEAddress,
        metadata: Option<Vec<u8>>,
    },
    /// Raw bytes that represent a valid transaction in EE
    Tx { payload: Vec<u8> },
    */
}

/// Type of message sent by ee to ol
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum OutboundMsg {
    /// Withdrawal is a special type of transfer to a specific system address in OL
    Withdrawal {
        destination: BitcoinAddress,
        /// Additional metadata that EE can attach to withdrawals, with short max length.
        /// Mainly to link withdrawal txid for block explorers but ee may use it differently.
        /// Should be opaque to OL/ASM and ignored.
        /// Disincentivise misuse through OL gas cost adjustment.
        metadata: Option<Vec<u8>>,
    },
    /*
    /// Native token (BTC) transfer to another account.
    Transfer {
        destination: Vec<u8>,
        metadata: Option<Vec<u8>>,
    },
    /// Raw txn posted to another account through OL.
    Tx { payload: Vec<u8> },
    */
}

/// Message sent by ee to ol
#[derive(Debug, Clone)]
pub struct OutboundMsgEnvelope {
    /// Account this message is for
    /// Equals bridge system account in withdrawals
    pub to: AccountAddress,
    /// Value transferred from this ee in this message
    pub value: BitcoinAmount,
    /// Message type and data
    pub msg: OutboundMsg,
}
