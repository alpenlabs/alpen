# CI Params Generation

Generates deployment params (`ol-params.json`, `asm-params.json`) using a prebuilt datatool image and the params scripts/templates from the checked-out repo ref. Manual runs validate that `datatool_image_commit` looks like a commit SHA, but they do not prove that the ECR tag came from a successful `ci-build.yml` run. A missing image tag still fails at `docker pull`.

## Templates

Each environment has a template directory (`templates/<env>/`) containing the two param files with:

- **Static values**: fields that are decided ahead of deployment and specific to the environment — operator pubkeys, admin pubkeys, sequencer predicate, denomination, recovery delay, confirmation depths, etc. These must be committed before running the workflow.
- **Placeholders** (`__FIELD_NAME__`): fields that depend on the code version or L1 state at deployment time — verification keys (VKs), genesis L1 anchor, `genesis_ol_blkid`, `inner_state`. These are filled by datatool during generation.

When adding a new environment or updating an existing one, commit the static values first. The workflow fills in the rest.

## Workflow

```
gh workflow run ci-genparams.yml --ref <branch> \
  -f env=<staging-v2|prod> \
  -f datatool_image_commit=<7-to-40-char-hex-sha> \
  -f genesis_l1_height=<height> \
  -f chain_config=<path-to-chainspec>
```

Dispatch ref:

| Argument | Description |
|-------|-------------|
| `--ref <branch>` | Branch/ref whose workflow file GitHub Actions runs. For normal runs, set this to the branch or commit to test and omit `checkout_ref`. |

Workflow inputs:

| Input | Description |
|-------|-------------|
| `datatool_image_commit` | Commit whose first 7 chars identify the prebuilt datatool image tag. Must be a 7-40 char lowercase hex SHA. |
| `checkout_ref` | Optional override for the repo ref checked out inside the job for params scripts/templates. Use only when the workflow file should come from `--ref`, but params scripts/templates should come from a different ref. |

When `checkout_ref` is omitted, the job checks out the workflow run commit (`github.sha`) for params scripts/templates. In the common case, this is the commit selected by `--ref`.

Download the artifact:
```
gh run download <run-id> -n params-<env>-<datatool-image-tag>
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
3. Create a template directory `templates/<env>/` with the two param files
4. Add the environment name to the `options` list in `ci-genparams.yml`

## Files

| File | Purpose |
|------|---------|
| `generate-params.sh` | Pulls datatool, extracts keys from templates, runs datatool, calls merge |
| `params-helper.py` | `extract-keys`: reads pubkeys from templates. `merge`: fills placeholders with datatool output |
| `templates/<env>/` | Per-environment param templates with static values + placeholders |
