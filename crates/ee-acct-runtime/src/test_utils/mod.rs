//! Test utilities for ee-acct-runtime.

pub mod dummy_ee;

pub use dummy_ee::{
    types::{DummyBlock, DummyBlockBody, DummyHeader, DummyPartialState, DummyTransaction},
    DummyExecutionEnvironment, DummyWriteBatch,
};
