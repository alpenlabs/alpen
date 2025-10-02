//! Test utilities for ee-acct-runtime.

pub mod dummy_ee;

pub use dummy_ee::{
    DummyExecutionEnvironment, DummyWriteBatch,
    types::{DummyBlock, DummyBlockBody, DummyHeader, DummyPartialState, DummyTransaction},
};
