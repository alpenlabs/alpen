# Admin action signing

Install the CLI:

```bash
cargo +nightly-2026-09-01 install --locked \
  --git https://github.com/alpenlabs/alpen.git \
  --branch feat/admin-tx-multisig-cli strata-test-cli
```

Each signer supplies the agreed full predicate, network, update type and authority
sequence number. Set `KEY_FILE` to a protected file containing your xpriv or
32-byte hex secret, readable only by you:

```bash
STRATA_ADMIN_KEY="$(cat "$KEY_FILE")" strata-test-cli sign-admin-action \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate "$PREDICATE"
```

The command displays the signing message and signer's public key on stderr, and
prints a base64 signature on stdout. Send the signature and public key to the
coordinator. The coordinator prefixes each signature
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

To cancel a pending update, supply its `UPDATE_ID`, original update type and
`PREDICATE`, and a new valid authority sequence number. Cancellation must confirm
before the update activates. Each signer runs:

```bash
STRATA_ADMIN_KEY="$(cat "$KEY_FILE")" strata-test-cli sign-admin-action \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate "$PREDICATE" --cancel-target-id "$UPDATE_ID"
```

The coordinator constructs the cancellation using the collected signatures:

```bash
strata-test-cli create-admin-cancel \
  --target-id "$UPDATE_ID" --update ee-stf-vk \
  --network bitcoin --magic "$MAGIC" --seq-no "$SEQNO" --predicate "$PREDICATE" \
  --signers-config "$ASM_PARAMS" \
  --signature "0:$SIGNATURE_A" --signature "2:$SIGNATURE_B" \
  --btc-url "$BTC_URL" --btc-user "$BTC_USER" --btc-password "$BTC_PASSWORD" \
  --btc-wallet "$BTC_WALLET" --fee-rate 2 --dry-run
```

Use `--update ol-stf-vk` or `--update asm-stf-vk` in both commands for OL or ASM
updates. Omit `--dry-run` to broadcast.

Run the Core integration check with:

```bash
python3 bin/strata-test-cli/tests/admin_action_smoke.py \
  --binary target/debug/strata-test-cli
```
