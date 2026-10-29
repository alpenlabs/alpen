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

To cancel an update, look up its reveal transaction through Bitcoin RPC:

```bash
strata-test-cli lookup-admin-update \
  --txid "$REVEAL_TXID" --asm-params "$GENESIS_ASM_PARAMS" \
  --btc-url "$BTC_URL" --btc-user "$BTC_USER" --btc-password "$BTC_PASSWORD"
```

The tool replays admin history from the deployed genesis ASM parameters to the
Bitcoin node's tip. It needs unpruned blocks since the ASM anchor and the matching
ASM rules; it stops if history enacts an ASM STF change. No OL RPC is needed.
The result includes `update_id`, full `predicate`, `status`, `activation_height`,
the authority's `last_seq_no`, and the scanned Bitcoin height and block ID.
Only cancel an update whose status is `pending`. Set `UPDATE_ID` and `PREDICATE`
from the result. Cancellation must confirm before activation.

Each signer signs the cancellation with the authority's next valid sequence number:

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
