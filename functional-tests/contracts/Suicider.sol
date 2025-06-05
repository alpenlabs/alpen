// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

//
// 2️⃣ Suicider Contract
//
contract Suicider {
    address payable public delegator;

    // Constructor receives the Delegator address
    constructor(address payable _delegator) payable {
        delegator = _delegator;
    }

    // Suicide method - anyone can call this to selfdestruct
    function suicide() external {
        selfdestruct(delegator);
    }
}
