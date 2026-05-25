// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Minimal counter for SP1 E2E — exercises contract deploy + interaction
/// within the proving window. Precompiled bytecode used in run-test.sh.
///
/// Compile: solc --bin --optimize Counter.sol
contract Counter {
    uint256 public count;
    event Incremented(uint256 newCount);

    function increment() external {
        count++;
        emit Incremented(count);
    }

    function get() external view returns (uint256) {
        return count;
    }
}
