// SPDX-License-Identifier: MIT
pragma solidity ^0.8.4;

/// @title ISubjectTransfer
/// @notice Canonical declarations for the inter-EE subject-transfer precompile.
///         The precompile lives at `SUBJECT_TRANSFER_PRECOMPILE_ADDRESS` and is
///         implemented natively (see `subject_transfer.rs`); this interface exists so
///         that on-chain callers and off-chain tooling can encode calldata and decode
///         logs or revert data.
///
/// @dev Calldata is `abi.encodePacked(bytes32 destAccount, bytes32 destSubject, bytes data)`.
///      On success the precompile burns the attached native value and emits
///      `SubjectTransferIntentEvent`. On rejected transfers it REVERTS (refunding unspent
///      gas) with ABI-encoded custom-error data: `bytes4(keccak256(signature))`.
///      Selectors (kept in sync with `subject_transfer.rs` by an in-crate keccak256 test):
///        IncorrectCallType()    0x7a5e63dc
///        MalformedCalldata()    0x59170bf0
///        NonIntegerAmount()     0xf7738c57
///        IncorrectAmount()      0x69640e72
///        OversizeAmount()       0xaf3af870
interface ISubjectTransfer {
    /// @notice Emitted when native value is burned for delivery to another EE account.
    /// @param amount Transfer amount in satoshis.
    /// @param sourceSubject Source subject derived from the EVM caller.
    /// @param destAccount Destination OL account.
    /// @param destSubject Destination subject inside the destination EE.
    /// @param transferData Opaque payload delivered with the transfer.
    event SubjectTransferIntentEvent(
        uint64 amount,
        bytes32 sourceSubject,
        bytes32 destAccount,
        bytes32 destSubject,
        bytes transferData
    );

    /// @notice The precompile was not reached via a direct CALL
    ///         (e.g. invoked through DELEGATECALL/CALLCODE/STATICCALL).
    error IncorrectCallType();

    /// @notice Calldata was too short to contain the destination account and subject.
    error MalformedCalldata();

    /// @notice The transfer value is not a whole number of satoshis.
    error NonIntegerAmount();

    /// @notice The transfer value is zero.
    error IncorrectAmount();

    /// @notice The transfer value exceeds `uint64` satoshis.
    error OversizeAmount();
}
