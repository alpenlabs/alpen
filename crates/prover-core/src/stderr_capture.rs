//! Capture of SP1 guest output emitted during simulation/proving.
//!
//! A hacky solution with a single purpose be able to see simulation
//! panic logs during the time we stabilize our proof statements.
//! Must be reconsidered and reassessed after TN3 deployment is complete.
//!
//! Some broader technical context:
//!
//! SP1's executor writes the guest's file descriptors 1 and 2 directly to the
//! host process's standard error via `eprintln!`, line-prefixed `stdout: ` and
//! `stderr: ` respectively (`sp1-core-executor` `syscalls/write.rs`). There is
//! no SDK, [`SP1Context`], or hook seam to intercept them: the writer fields on
//! the context are dead code, the SDK's `stdout()`/`stderr()` builders are
//! commented out, and fd 1/2 are short-circuited before the hook table. The
//! only place the bytes exist is the host's fd 2.
//!
//! [`capture`] brackets a closure by redirecting OS fd 2 into a pipe, draining
//! it on a helper thread, then restoring fd 2 and returning the captured bytes.
//! [`tee_to_tracing`] re-emits each captured line as a `tracing` event under
//! the `sp1.guest` target, so guest output inherits the active `prove{task=…}`
//! span instead of vanishing into raw container logs.
//!
//! This is the alpen-side ("broad window") home for the capture: it wraps the
//! whole `strategy.prove` call in [`crate::prover`], so it sees output from
//! both the simulation pre-flight and the local prover without touching
//! zkaleido. The narrower alternative captures inside zkaleido's executor.
//!
//! # Cross-talk: what else can land in the pipe
//!
//! There is exactly **one** fd-2 slot per process, and [`CAPTURE_LOCK`] only
//! serializes *our own* capture windows against each other — it does **not**
//! stop other threads from writing to fd 2. So anything any thread writes to
//! stderr while the window is open is diverted into our pipe: it is captured
//! and re-emitted (see [`tee_to_tracing`]) rather than lost, but it does not
//! reach the real stderr in real time, surfaces *after* the window, and — since
//! it lacks SP1's `stdout: `/`stderr: ` prefix — is relabeled as `sp1.guest`.
//!
//! In practice this is a small risk **in this deployment**, because the things
//! that would otherwise collide do not go to fd 2:
//!
//! - `strata-logging`'s structured logs go to **stdout (fd 1)** (the `tracing_subscriber::fmt`
//!   layer's default writer) and/or a **file**, never to stderr. So routine `tracing` output from
//!   concurrent tasks is *not* captured by this window. (This holds only while that layer keeps its
//!   stdout/file writer — if it is ever switched to `with_writer(io::stderr)`, the cross-talk
//!   returns.)
//! - The realistic residue is therefore a **concurrent thread's panic banner** (Rust's default
//!   panic hook prints `thread '…' panicked at …` to stderr) or an explicit `eprintln!`/direct
//!   `io::stderr()` write — both rare during a prove, and the former already signals an abnormal
//!   event.
//!
//! # Other caveats
//!
//! - guests are typically `panic = "abort"`, so a panic yields the message and location, not a full
//!   backtrace.
//! - capture is best-effort: any failure to set up the redirection runs the closure with stderr
//!   untouched (capturing empty buffer) and would result in not capturing the simulation's stderr.

use std::{
    fs::File,
    io::{stderr, Read, Write},
    os::fd::FromRawFd,
    sync::Mutex,
    thread,
};

use tracing::{info, warn};

/// Serializes fd-2 redirection across threads (the redirect is process-global).
static CAPTURE_LOCK: Mutex<()> = Mutex::new(());

/// `tracing` target for re-emitted guest output.
const GUEST_TARGET: &str = "sp1.guest";

/// Runs `f` with the host's standard error (fd 2) redirected into a pipe and
/// returns `f`'s result alongside the bytes written to fd 2 during the call.
///
/// Best-effort: if the redirection cannot be set up, `f` runs with stderr
/// untouched and the captured buffer is empty.
pub(crate) fn capture<R>(f: impl FnOnce() -> R) -> (R, Vec<u8>) {
    let _guard = CAPTURE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Flush so previously buffered host stderr is not pulled into the pipe.
    let _ = stderr().flush();

    let Some(mut redirect) = Redirect::install() else {
        return (f(), Vec::new());
    };

    let result = f();
    let captured = redirect.finish();
    (result, captured)
}

/// Re-emits captured guest output as `tracing` events under [`GUEST_TARGET`].
///
/// Lines tagged `stderr: ` (guest panics and stderr) are emitted at WARN;
/// `stdout: ` lines (guest `println!`) at INFO. The prefixes are added by SP1's
/// executor. Call this only after fd 2 is restored, so events flow to the real
/// subscriber rather than back into the capture pipe.
pub(crate) fn tee_to_tracing(captured: &[u8]) {
    if captured.is_empty() {
        return;
    }
    for line in String::from_utf8_lossy(captured).lines() {
        if let Some(rest) = line.strip_prefix("stderr: ") {
            warn!(target: GUEST_TARGET, stream = "stderr", line = %rest);
        } else if let Some(rest) = line.strip_prefix("stdout: ") {
            info!(target: GUEST_TARGET, stream = "stdout", line = %rest);
        } else {
            // Unprefixed output: another host stderr writer captured in the
            // window, or a future SP1 format. Surface it verbatim.
            warn!(target: GUEST_TARGET, line = %line);
        }
    }
}

/// RAII fd-2 redirection.
///
/// [`Redirect::install`] points fd 2 at a fresh pipe and spawns a reader thread
/// draining it; [`Redirect::finish`] restores fd 2 and returns the drained
/// bytes. [`Drop`] restores fd 2 best-effort if `finish` was not called (e.g. a
/// panic unwound through the captured closure), so stderr is never left
/// dangling on the pipe.
struct Redirect {
    /// `dup` of the original fd 2, restored over fd 2 on finish/drop. `-1` once
    /// consumed.
    saved_fd: i32,
    /// Reader thread draining the pipe; joined to recover the captured bytes.
    reader: Option<thread::JoinHandle<Vec<u8>>>,
}

impl Redirect {
    fn install() -> Option<Self> {
        // SAFETY: raw fd syscalls. Return values are checked; each fd is closed
        // exactly once (read end by the reader thread's `File`, write end here,
        // saved end on restore).
        unsafe {
            let saved_fd = libc::dup(libc::STDERR_FILENO);
            if saved_fd < 0 {
                return None;
            }
            let mut fds = [0i32; 2];
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                libc::close(saved_fd);
                return None;
            }
            let [read_fd, write_fd] = fds;

            // Drain on a helper thread so a guest that out-writes the pipe
            // buffer cannot block on a full pipe.
            let reader = thread::spawn(move || {
                let mut pipe = File::from_raw_fd(read_fd);
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                buf
            });

            if libc::dup2(write_fd, libc::STDERR_FILENO) < 0 {
                // Closing write_fd leaves the pipe with no writer, so the reader
                // sees EOF and exits on its own.
                libc::close(write_fd);
                libc::close(saved_fd);
                return None;
            }
            // fd 2 now dups the write end; drop our own handle so the pipe's
            // only writer is fd 2 itself (closed on restore → reader EOF).
            libc::close(write_fd);

            Some(Self {
                saved_fd,
                reader: Some(reader),
            })
        }
    }

    /// Restores fd 2 and returns the captured bytes.
    fn finish(&mut self) -> Vec<u8> {
        self.restore();
        match self.reader.take() {
            Some(handle) => handle.join().unwrap_or_default(),
            None => Vec::new(),
        }
    }

    fn restore(&mut self) {
        let _ = stderr().flush();
        if self.saved_fd >= 0 {
            // SAFETY: `saved_fd` is a valid dup of the original stderr.
            // Restoring it reassigns fd 2 and closes the pipe's last write end,
            // signalling EOF to the reader.
            unsafe {
                libc::dup2(self.saved_fd, libc::STDERR_FILENO);
                libc::close(self.saved_fd);
            }
            self.saved_fd = -1;
        }
    }
}

impl Drop for Redirect {
    fn drop(&mut self) {
        self.restore();
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes a line to fd 2 the way SP1's executor effectively does at
    /// runtime — directly through the [`std::io::Stderr`] handle, *not* via the
    /// `eprintln!` macro. The macro path is intercepted by libtest's
    /// thread-local output capture and would never reach fd 2 under test.
    fn write_fd2(line: &str) {
        let mut err = stderr();
        let _ = writeln!(err, "{line}");
    }

    #[test]
    fn captures_stderr_writes() {
        let (ret, captured) = capture(|| {
            write_fd2("stderr: boom at lib.rs:1");
            write_fd2("stdout: hello");
            42
        });
        assert_eq!(ret, 42);
        let text = String::from_utf8_lossy(&captured);
        assert!(text.contains("stderr: boom at lib.rs:1"), "got: {text}");
        assert!(text.contains("stdout: hello"), "got: {text}");
    }

    #[test]
    fn restores_stderr_after_capture() {
        // After a capture window, fd 2 must be a normal stderr again: a second
        // capture still works, proving the first restored cleanly.
        let _ = capture(|| write_fd2("stderr: first"));
        let (_, captured) = capture(|| write_fd2("stderr: second"));
        let text = String::from_utf8_lossy(&captured);
        assert!(text.contains("second"), "got: {text}");
        assert!(!text.contains("first"), "leaked across windows: {text}");
    }
}
