// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title Selfdestruct Contract with receiver.
/// @notice Demonstrates updating state and self-destruction of the contract
contract SelfDestructToAddress {
    address payable public receiver;

    // Constructor receives the address to send the funds to on sefldestruct.
    constructor(address payable _receiver) payable {
        receiver = _receiver;
    }

    // Suicide method - anyone can call this to selfdestruct
    function suicide() external {
        selfdestruct(receiver);
    }
}
