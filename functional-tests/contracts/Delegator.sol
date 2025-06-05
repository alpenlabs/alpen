// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

//
// 1️⃣ Delegator Contract
//
contract Delegator {
    uint256 public counter;

    // Optional - you could add methods to modify the counter later
    function increment() external {
        counter += 1;
    }
}
