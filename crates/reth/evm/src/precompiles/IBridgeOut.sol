// SPDX-License-Identifier: MIT
pragma solidity ^0.8.4;

/// @title IBridgeOut
/// @notice Canonical declarations for the bridge-out precompile's custom errors.
///         The precompile lives at `BRIDGEOUT_PRECOMPILE_ADDRESS` and is implemented
///         natively (see `bridge.rs`); this interface exists so that on-chain callers
///         and off-chain tooling can decode its revert data.
///
/// @dev On a rejected withdrawal the precompile REVERTS (refunding unspent gas) with
///      ABI-encoded custom-error data: `bytes4(keccak256(signature)) ++ abi.encode(args)`.
///      Selectors (kept in sync with `bridge.rs` by an in-crate keccak256 test):
///        IncorrectCallType()            0x7a5e63dc
///        MalformedCalldata()            0x59170bf0
///        MalformedCalldataBosd()        0xc8e45892
///        OversizeBosd(uint256)          0x2725ac73
///        NonIntegerAmount()             0xf7738c57
///        IncorrectAmount(uint256)       0x88967d2f
///        OversizeWithdrawal(uint256)    0xb0701377
interface IBridgeOut {
    /// @notice The precompile was not reached via a direct CALL
    ///         (e.g. invoked through DELEGATECALL/CALLCODE/STATICCALL).
    error IncorrectCallType();

    /// @notice Calldata was too short to contain the 4-byte operator selector
    ///         followed by at least one BOSD byte.
    error MalformedCalldata();

    /// @notice The BOSD bytes are not a valid descriptor.
    error MalformedCalldataBosd();

    /// @notice The BOSD descriptor length exceeds the configured maximum.
    /// @param max The maximum allowed descriptor length, in bytes.
    error OversizeBosd(uint256 max);

    /// @notice The withdrawal value is not a whole number of satoshis.
    error NonIntegerAmount();

    /// @notice The withdrawal value is zero or not a positive multiple of the denomination.
    /// @param denomination The withdrawal denomination, in satoshis.
    error IncorrectAmount(uint256 denomination);

    /// @notice The withdrawal value exceeds the maximum permitted withdrawal.
    /// @param max The maximum permitted withdrawal, in satoshis.
    error OversizeWithdrawal(uint256 max);
}
