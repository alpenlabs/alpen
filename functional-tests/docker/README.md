# Functional Tests in Docker

Run functional tests inside a resource-limited Docker container.

## Usage

```bash
# Build (first time or after Rust code changes — slow; cached after that)
docker compose build

# Run a test
docker compose run --rm functional-tests -t test_node_version

# Run a group
docker compose run --rm functional-tests -g bridge

# List all tests
docker compose run --rm functional-tests --list
```

## Resource Limits

Edit `docker-compose.yml` to adjust CPU and memory:

```yaml
deploy:
  resources:
    limits:
      cpus: "0.5"    # decimal cores (0.1 = 10% of one core)
      memory: 512MB
```

Test logs are persisted to `../_dd/` on the host via volume mount.
