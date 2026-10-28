# Offline admin signing

No proposal file is needed. Agree on the action, network, current valid sequence,
predicate type and condition hash. For cancellations, also agree on the target ID.
`--predicate-hash` is SHA-256 of the decoded condition bytes, not the predicate file
or necessarily the program VK. Hash-only input supports Sp1Groth16 conditions >32
bytes; the existing `--predicate` input still prints messages for other predicates.

```bash
strata-test-cli print-signing-message \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate-type Sp1Groth16 --predicate-hash "$CONDITION_HASH"
```

Sign that exact text with the authority's address in your existing wallet. Stdout
has LF line endings and no trailing newline; the digest on stderr is a cross-check.
For local keys, pipe the xpriv or 32-byte hex secret into `sign-admin-action` with
those same action arguments plus `--signers-config "$ASM_PARAMS" --key-stdin`.
It verifies key membership and emits `<index>:<base64>` without Bitcoin RPC.

Give the collected indexed signatures to the existing transaction command through
repeated `--signature` options. That command requires the full `--predicate` and
verifies its canonical message and quorum **before funding**. Its `--dry-run`
constructs the signed transaction pair without broadcasting. Bitcoin RPC, wallet,
fees and commit amount are passed separately through `BitcoinFundingConfig`;
`PredicateUpdateRequest` contains no funding settings. Existing CLI flags remain.
Check current on-chain authority/sequence state separately; a params file may be
stale. The message signs the protocol action, not funding inputs, fees or an expiry.

Validation: `cargo test -p strata-test-cli --locked`, package Clippy with
`--all-targets --locked -- -D warnings`, and `cargo fmt --all --check`.
