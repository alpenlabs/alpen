use std::fmt;

use arbitrary::Arbitrary;
use borsh::{BorshDeserialize, BorshSerialize};
use digest::Digest;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use strata_checkpoint_types::SignedCheckpoint;
use strata_identifiers::{BitcoinAmount, Buf32};
use strata_primitives::l1::OutputRef;
