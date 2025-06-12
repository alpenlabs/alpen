//! Shim to ensure that we *always* print panic stack traces.
//!
//! This is not actually generally useful.  Only set it if actually necessary.

use std::{backtrace, panic};

use tracing::*;

fn handle_panic(info: &panic::PanicHookInfo<'_>) {
    let bt = backtrace::Backtrace::force_capture();

    match info.payload().downcast_ref::<&str>() {
        Some(reason) => print_panic(&bt, Some(reason)),
        None => match info.payload().downcast_ref::<String>() {
            Some(reason) => print_panic(&bt, Some(reason.as_ref())),
            None => print_panic(&bt, None),
        },
    }

    // TODO
}

fn print_panic(bt: &backtrace::Backtrace, reason: Option<&str>) {
    match reason {
        Some(reason) => error!(%reason, "thread panicking\n{bt}"),
        None => error!("thread panicking\n{bt}"),
    }
}

/// Sets a panic hook to (1) print the backtrace and (2) call the previous hook.
pub fn set_panic_hook() {
    let old_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        handle_panic(info);
        old_hook(info);
    }));
}
