//! Tests for snark account operations including verification and state transitions

#[cfg(test)]
mod validation;

#[cfg(test)]
mod inbox;

#[cfg(test)]
mod updates;

#[cfg(test)]
mod deposit_withdrawal;

#[cfg(test)]
mod multi_operations;

#[cfg(test)]
mod edge_cases;

#[cfg(test)]
mod ledger_references;

#[cfg(test)]
mod stf;
