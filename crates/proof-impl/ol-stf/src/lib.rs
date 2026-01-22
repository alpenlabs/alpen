#[cfg(not(target_os = "zkvm"))]
pub mod program;

mod statements;

pub use statements::process_ol_stf;
