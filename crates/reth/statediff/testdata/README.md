# BatchStateDiff Golden Fixtures

These binary fixtures pin the current `BatchStateDiff` wire format used for EE DA.

Fixtures:

- `empty_batch.bin`: empty diff with no accounts, storage, or bytecode.
- `single_account_create.bin`: one created account with no storage.
- `storage_only_update.bin`: storage-only updates for an existing account.
- `selfdestruct_recreate.bin`: a selfdestruct followed by recreate within the same batch.
- `code_hash_and_bytecode_update.bin`: a code-hash update paired with deployed bytecode payload.

Compatibility window:

- These fixtures intentionally freeze the current encoding shape.
- Updating a fixture means the wire format changed on purpose.
