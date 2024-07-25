#![allow(dead_code)] // TODO: remove once `WithdrawData` is used
//! Infrastructure for controlling EVM execution.  This operates on similar
//! principles to the Ethereum engine API used for CL clients to control their
//! corresponding EL client.

pub mod engine;
pub mod messages;
pub mod stub;

pub mod errors;
