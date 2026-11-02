These files contain serialized BIP-340 predicates for the native checkpoint
programs:

| Spec | Predicate file | Public test signing key |
|------|----------------|-------------------------|
| V1 | `checkpoint-bip340-schnorr-test.predicate` | `[0x01; 32]` |
| V2 | `checkpoint-v2-bip340-schnorr-test.predicate` | `[0x03; 32]` |

V1 preserves the `bip340-schnorr-test` value previously derived by datatool.
V2 uses a distinct key so routing a proof to the V1 host fails verification,
and avoids the mock-EE snark-account key (`[0x02; 32]`). V0 shares V1's key for
compatibility, but has no registered native checkpoint program.

The registry unit test checks that these fixtures match the per-spec predicates.
The upgrade functional test checks that checkpoints finalize across the key change.
