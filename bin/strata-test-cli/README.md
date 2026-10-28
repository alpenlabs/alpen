# Admin action signing (DO NOT MERGE)

The offline commands extend `feat/admin-tx-multisig-cli`. They take command-line
values and return signature strings; no proposal or signature file is required.

Admin owners authorize a canonical Bitcoin `signmessage` message. The funding
wallet later signs the Bitcoin transaction. These are separate operations.

## 1. Agree on the action

Select the update kind, Bitcoin network, current valid sequence number, predicate
type and predicate condition hash. For a cancellation, also agree on the queued
update ID and the exact predicate being cancelled.

`--predicate-hash` is SHA-256 of the decoded **condition bytes**. It is not the
hash of a `.predicate` file, the `Sp1Groth16:` text, or necessarily the program VK.
Hash-only input currently supports `Sp1Groth16` conditions longer than 32 bytes.
For short conditions or other types, supply the full `--predicate` instead.

The commands do not query live admin sequence numbers or authority rotations.
Check those before collecting real authorizations. The trusted ASM params file
must match the effective authority, not simply an obsolete genesis configuration.

## 2. Display the exact signing message

```bash
strata-test-cli print-signing-message \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate-type Sp1Groth16 --predicate-hash "$CONDITION_HASH"
```

Stdout contains only the canonical message: LF line endings, two spaces before
detail fields, and no trailing newline. Review context and the message-signing
digest go to stderr. Sign the text in Sparrow/Electrum using the address belonging
to this authority. The digest is a cross-check, not the message to paste.

The existing full-predicate invocation still works:

```bash
strata-test-cli print-signing-message \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate "$FULL_PREDICATE"
```

## 3. Optional local signing

Owners who already keep their key in a local protected file can use:

```bash
strata-test-cli sign-admin-action \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate-type Sp1Groth16 --predicate-hash "$CONDITION_HASH" \
  --signers-config "$ASM_PARAMS" --key-file "$PRIVATE_KEY_PATH"
```

Alternatively use `--key-stdin` and supply the key through stdin. Exactly one key
source is required. Neither option puts the key itself in argv. Key input is an
xpriv (its own private key, without implicit child derivation) or 32-byte hex.
The derived address must match a signer in the action's required authority.

The command displays the canonical action on stderr and emits only
`<signer_index>:<base64 BIP-137 signature>` on stdout. The index is derived from
the key and the configured signer order. Send that string to the coordinator.
There is no Bitcoin RPC connection, funding operation or transaction broadcast.
Hardware-wallet owners can stay with their existing message-signing interface.

## 4. Coordinator verifies the collected signatures offline

```bash
strata-test-cli verify-admin-signatures \
  --update ee-stf-vk --network bitcoin --seq-no "$SEQNO" \
  --predicate-type Sp1Groth16 --predicate-hash "$CONDITION_HASH" \
  --signers-config "$ASM_PARAMS" \
  --signature "$SIGNATURE_A" --signature "$SIGNATURE_B"
```

Each input may be a bare base64 wallet signature or an indexed signature. Bare
signatures are matched to the configured signer by ASM's verifier. The JSON
report includes normalized indexed `signatures`, valid indices, required count,
the exact message and `threshold_verified`. Duplicate or invalid signatures fail.
An incomplete threshold fails unless `--allow-partial` is explicitly supplied;
in that mode the report shows `threshold_verified: false`.

## 5. Construct and fund later, with the full predicate

Pass the normalized strings to the existing `create-ee-predicate-update`,
`create-checkpoint-predicate-update`, `create-asm-predicate-update`, or
`create-admin-cancel` command using repeated `--signature` options. Those commands
still require the full `--predicate`, the funding wallet and Bitcoin RPC options.
They recompute the canonical message from the full predicate and verify the
signatures **before** selecting funding UTXOs or constructing transactions. A
different condition, action, network, sequence or cancel target fails that check.

Their existing `--dry-run` builds and prints the signed transaction pair without
broadcasting; it still uses the funding wallet. It is separate from the new
offline signing and verification commands.

## Bitcoin configuration boundary

`PredicateUpdateRequest` contains authorization fields and the protocol chain
context. It carries no RPC URL/credentials, fee rate, funding wallet, commit output
amount or dry-run setting.

At the CLI boundary, the existing Bitcoin flags populate `BitcoinFundingConfig`
in `admin_bitcoin.rs`. The execution function receives that configuration
separately, verifies the admin signatures, and only then constructs a
`BitcoinFunding` runtime context. Funding and broadcast functions live behind
that context; signing never receives it. `dry_run` is a separate execution choice.
The Bitcoin network remains in the action request because it is part of the
signed protocol message. No environment or configuration-file source is imposed;
those can supply the same funding configuration independently of the action.

## Compatibility and validation

The signing text covers protocol version, action, required role, network,
sequence and predicate details (plus target ID for cancels). It does not bind
the funding UTXOs, fees, SPS-50 magic, signer-config file or an expiry time.
Collecting signatures creates real authorizations; a local label such as
“rehearsal” does not change their meaning.

The pinned ASM v1 API can render only full predicates. The hash-only adapter is
therefore deliberately restricted to predicate updates/cancels and guarded
against protocol version changes. Tests compare its text and digest byte-for-byte
with ASM's renderer for all three targets, cancellations, networks and several
condition sizes. The full-predicate transaction path uses ASM's renderer directly.

Validation:

```bash
cargo test -p strata-test-cli --locked
cargo clippy -p strata-test-cli --all-targets --locked -- -D warnings
cargo fmt --all --check
cargo build -p strata-test-cli --locked
python3 bin/strata-test-cli/tests/admin_action_smoke.py \
  --binary target/debug/strata-test-cli
```

The regtest smoke test requires `bitcoind` and `bitcoin-cli`. With Core 31 it
verified an independently produced Core message signature, local signing,
automatic signer indices, partial and duplicate rejection, and full-predicate
transaction construction. Core accepted the commit/reveal pair through
`testmempoolaccept`; the test broadcasts neither transaction. It creates an isolated
node and removes its temporary state after shutdown. It does not test live ASM
sequence state or activation.

No protocol or dependency changes are needed. This branch is for review and must
not be merged or used for a mainnet ceremony before that review.
