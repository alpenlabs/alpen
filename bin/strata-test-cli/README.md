# Admin action signing

Install the CLI:

```bash
cargo +nightly-2026-09-01 install --locked \
  --git https://github.com/alpenlabs/alpen.git \
  --branch feat/admin-action-signing strata-test-cli
```

Each signer supplies the agreed full predicate, network, update type and authority
sequence number. Set `STRATA_ADMIN_KEY` to the signer's xpriv or 32-byte hex secret:

```bash
read -r -s STRATA_ADMIN_KEY
export STRATA_ADMIN_KEY
strata-test-cli sign-admin-action \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate "$PREDICATE"
unset STRATA_ADMIN_KEY
```

The command prints the signing message to stderr and a base64 signature to stdout.
Send that signature to the coordinator. The coordinator prefixes each signature
with its index in the deployed authority's signer set and constructs the transaction:

```bash
strata-test-cli create-ee-predicate-update \
  --network bitcoin --magic "$MAGIC" --seq-no "$SEQNO" --predicate "$PREDICATE" \
  --signers-config "$ASM_PARAMS" \
  --signature "0:$SIGNATURE_A" --signature "2:$SIGNATURE_B" \
  --btc-url "$BTC_URL" --btc-user "$BTC_USER" --btc-password "$BTC_PASSWORD" \
  --btc-wallet "$BTC_WALLET" --fee-rate 2 --dry-run
```

The command verifies the signatures and threshold before funding, then prints the
signed commit/reveal transactions. Omit `--dry-run` to broadcast. Use
`create-checkpoint-predicate-update` or `create-asm-predicate-update` for OL or ASM
updates, with the corresponding `--update` value when signing.

Run the Core integration check with:

```bash
python3 bin/strata-test-cli/tests/admin_action_smoke.py \
  --binary target/debug/strata-test-cli
```
