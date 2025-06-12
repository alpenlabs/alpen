// SPDX-License-Identifier: UNLICENSED
pragma solidity ^0.8.0;

/// @title Selfdestruct Contract with sending to self.
/// @notice Demonstrates updating state and self-destruction of the contract
contract SelfDestruct {
    uint256 public state;

    /// @notice Increments the `state` variable by 1
    function updateState() external {
        state += 1;
    }

    /// @notice Destroys the contract and sends any remaining Ether to itself
    function destroyContract() external {
        selfdestruct(payable(address(this)));
    }
}
