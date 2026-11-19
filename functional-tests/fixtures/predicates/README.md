These files contain serialized BIP-340 predicates for the native checkpoint
programs:

| Spec | Predicate file | Public test signing key |
|------|----------------|-------------------------|
| V1 | `checkpoint-bip340-schnorr-test.predicate` | `[0x01; 32]` |
| V2 | `checkpoint-v2-bip340-schnorr-test.predicate` | `[0x03; 32]` |
| V3 | `checkpoint-v3-bip340-schnorr-test.predicate` | `[0x04; 32]` |

V1 preserves the `bip340-schnorr-test` value previously derived by datatool.
Each later spec uses its own key so routing a proof to another spec's host
fails verification. The keys skip the mock-EE snark-account key (`[0x02; 32]`).
V0 shares V1's key for compatibility, but has no registered native checkpoint
program.

The registry unit test checks that these fixtures match the per-spec predicates.
The upgrade functional tests check that checkpoints finalize across the V1 to
V2 and V2 to V3 key changes.
