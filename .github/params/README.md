# CI Params Generation

Generates deployment params (`alpen-params.json`, `ol-params.json`, `asm-params.json`) using the prebuilt datatool image from a given commit.

## Templates

Each environment has a template directory (`templates/<env>/`) containing the param files with:

- **Static values**: fields that are decided ahead of deployment and specific to the environment — operator pubkeys, admin pubkeys, sequencer predicate, denomination, recovery delay, confirmation depths, etc. These must be committed before running the workflow.
- **Placeholders** (`__FIELD_NAME__`): fields that depend on the code version or L1 state at deployment time — verification keys (VKs), genesis L1 anchor, `genesis_ol_blkid`, `inner_state`. These are filled by datatool during generation.

When adding a new environment or updating an existing one, commit the static values first. The workflow fills in the rest.

## Workflow

```
gh workflow run ci-genparams.yml --ref <branch> \
  -f env=<staging-v2|prod> \
  -f commit=<short-sha> \
  -f genesis_l1_height=<height> \
  -f chain_config=<path-to-chainspec>
```

Download the artifact:
```
gh run download <run-id> -n params-<env>-<commit>
```

## GitHub Environment Setup

Each environment in the workflow input (`staging-v2`, `prod`) must have a matching GitHub environment with these configured:

| Name | Type | Description |
|------|------|-------------|
| `SHARED_ECR_ROLE_ARN` | Secret | IAM role ARN for OIDC ECR access |
| `PARAMS_BTC_RPC_URL` | Secret | Bitcoin RPC endpoint (read-only fullnode, no wallet needed) |
| `PARAMS_BTC_RPC_USER` | Secret | Bitcoin RPC username |
| `PARAMS_BTC_RPC_PASSWORD` | Secret | Bitcoin RPC password |

Each environment connects to its own bitcoin node. The node only serves `getblock`/`getblockheader` RPCs for genesis L1 anchor generation.

To add a new environment:
1. Create the GitHub environment (Settings → Environments) with the exact name used in the workflow input
2. Add the secrets above
3. Create a template directory `templates/<env>/` with the param files
4. Add the environment name to the `options` list in `ci-genparams.yml`

## Files

| File | Purpose |
|------|---------|
| `generate-params.sh` | Pulls datatool, extracts keys from templates, runs datatool, calls merge |
| `params-helper.py` | `extract-keys`: reads pubkeys from templates. `merge`: fills placeholders with datatool output |
| `templates/<env>/` | Per-environment param templates with static values + placeholders |
