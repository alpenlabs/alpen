# PR #1004 (alpenlabs/alpen) — Fix Plan

## Summary

- Goal: add `test-mode` cargo feature to disable passwords and accept a direct seed setting.
- Reviewer feedback:
  - Unnecessary `Arc` usage; simplify ownership/borrowing (from @storopoli).
  - Add substantially more rustdoc across new/changed items (from @Zk2u).
  - SecretStore trait design questions deferred to @Zk2u.
- Bots:
  - Codecov: patch coverage 0% (≈23 lines) with gaps in CLI modules.

## Required Changes

- Documentation:
  - Add rustdoc to all new public items and modules in CLI/seed paths.
  - Document security caveats: `test-mode` is unsafe for production and should only be used in tests/dev.
- Code hygiene:
  - Remove unnecessary `Arc` layering where values are immutable or single-owner.
  - Prefer borrowing (`&T`) or moving `T` over `Arc<T>`; only introduce `Arc` at concurrency boundaries.
  - Revisit `SecretStore` trait: ensure clear responsibilities, correct bounds, and ergonomics (see notes below).
- Feature gating & safety:
  - Guard test-only code with `#[cfg(feature = "test-mode")]` and avoid accidental inclusion in release builds.
  - Consider a compile-time denial in release: `#[cfg(all(not(test), not(feature = "test-mode")))]` paths should remain unaffected.
  - Make the feature name consistent across crates (`test-mode` vs `test_mode`) and propagate via workspace features if needed.
- CLI UX:
  - Validate seed value format early; emit clear errors and exit codes.
  - Ensure `--seed`/setting is mutually compatible with existing flags; document precedence.

## Test Coverage Targets (from Codecov comment)

Focus on unit tests for these files to address the 0% patch coverage signal:

- bin/alpen-cli/src/seed/seed_provider.rs
  - Deterministic seed when provided via setting/flag.
  - Fallback behavior when absent; invalid seed formats (length/hex errors).
  - Error surfaces are tested (Result errors, messages).
- bin/alpen-cli/src/main.rs
  - CLI arg parsing for `--seed` and `--test-mode` feature interactions.
  - Non-zero exit on invalid inputs.
- bin/alpen-cli/src/settings.rs
  - Settings merge/load (defaults, env overrides, CLI precedence).
  - Serialization/deserialization of the new seed field.
- bin/alpen-cli/src/cmd/reset.rs
  - Reset command behavior in test-mode; confirmation flows.
- bin/alpen-cli/src/seed.rs
  - Command path happy-path and error-path coverage.

## Implementation Notes

- Removing unnecessary `Arc`:
  - If a value is immutable and not shared across threads, pass by reference `&T` or move `T`.
  - If cloning is cheap, prefer `T: Clone` over `Arc<T>`.
  - Introduce `Arc<T>` only at the API boundary that requires cross-thread sharing; avoid propagating `Arc` through the entire call graph.
  - Avoid nested sync types (e.g., `Arc<Mutex<Option<T>>>`); prefer `Option<Arc<T>>` or plain `Option<T>` when synchronization isn’t required.
- SecretStore trait sketch:
  - Split responsibilities if needed: e.g., `SecretReader` and `SecretWriter` traits; or keep a single trait with clear methods.
  - Ensure `Send + Sync + 'static` where used across async/tasks; return types that don’t leak secrets via `Debug`.
  - Prefer fixed-size types for seeds (e.g., `[u8; 32]` or a `Seed` newtype) to enforce invariants.
  - Provide blanket impls or adapters where convenient; add rustdoc with examples.
- Rustdoc checklist:
  - Add module-level docs explaining `test-mode` and security implications.
  - Document every public struct/enum/trait/method added or modified.
  - Include minimal examples for CLI usage and programmatic usage where applicable.

## Suggested Tasks Checklist

- [ ] Replace unnecessary `Arc` with owned/borrowed types; keep `Arc` only at concurrency boundaries.
- [ ] Finalize `SecretStore` trait API and document it thoroughly.
- [ ] Add rustdoc to all public items touched by the PR.
- [ ] Add targeted unit tests for the five CLI files listed above.
- [ ] Verify `test-mode` feature gating and naming across crates.
- [ ] Update CLI help and README/CHANGELOG snippets to reflect new flags/settings.
- [ ] Re-run CI; confirm Codecov patch coverage > 80% for the touched files.

## References

- PR: https://github.com/alpenlabs/alpen/pull/1004
- Reviewers: @storopoli (CHANGES_REQUESTED), @Zk2u (CHANGES_REQUESTED)
- Codecov report (files with missing lines): CLI seed/settings/main/reset modules
