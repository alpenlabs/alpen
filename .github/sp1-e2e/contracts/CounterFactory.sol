// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

/// Deploys a Counter and calls it in the same transaction.
/// Exercises the same-block deploy+call path where revm uses
/// journaled state instead of code_by_hash_ref.
contract CounterFactory {
    event Deployed(address counter, uint256 value);

    function deployAndCall() external returns (address, uint256) {
        Counter c = new Counter();
        c.increment();
        c.increment();
        c.increment();
        uint256 val = c.get();
        emit Deployed(address(c), val);
        return (address(c), val);
    }
}

contract Counter {
    uint256 public count;

    function increment() external {
        count++;
    }

    function get() external view returns (uint256) {
        return count;
    }
}
