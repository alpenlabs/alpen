# Admin action signing

Print the canonical Bitcoin `signmessage` text for an admin predicate update:

```bash
strata-test-cli print-signing-message \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate-type Sp1Groth16 --predicate-hash "$CONDITION_HASH"
```

Both `print-signing-message` and `sign-admin-action` accept either `--predicate`
with the full predicate or `--predicate-type` with `--predicate-hash`. The hash is
SHA-256 of the decoded condition bytes; hash input supports Sp1Groth16 conditions
longer than 32 bytes. Use `--cancel-target-id` to sign a cancellation.

Sign the exact stdout text with the authority's address in a wallet. It has no
trailing newline. To use a local key, pipe its xpriv or 32-byte hex secret into
`sign-admin-action` with the same action arguments and
`--signers-config "$ASM_PARAMS" --key-stdin`. The command checks membership and
prints `<index>:<base64>`.

Pass indexed signatures through repeated `--signature` options to the transaction
command. It requires the full `--predicate` and verifies quorum before funding.
Use `--dry-run` to construct the commit/reveal pair without broadcasting.

Run the Bitcoin Core integration check with:

```bash
python3 bin/strata-test-cli/tests/admin_action_smoke.py \
  --binary target/debug/strata-test-cli
```
