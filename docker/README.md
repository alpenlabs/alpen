# Running Locally

## Running OL + EL with the shared compose

This setup builds both `strata` (OL) and `alpen-client` (EL) from the repo Dockerfiles and wires them to a local bitcoind.

1. Ensure config files exist:
   - `docker/configs/config.toml`
   - `docker/configs/params.json`
2. From repo root, start the stack:
   ```bash
   docker compose -f docker/docker-compose-ol-el.yml up -d --build
   ```
3. Services:
   - `strata` listens on `8432` and reads the mounted config/params at `/config/config.toml` and `/config/params.json`.
   - `alpen-client` exposes `8545/8546/30303` and is pointed at `ws://strata:8432`.
4. Stop the stack when done:
   ```bash
   docker compose -f docker/docker-compose-ol-el.yml down
   ```

### Run OL only (silo)
- Prereq: bitcoind must be up; compose handles it automatically.
- Start only OL + bitcoind:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml up -d --build bitcoind strata
  ```
- Stop:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml down
  ```

### Run EL only (silo)
- Assumes an OL endpoint is available at `--ol-client-url` (update the command in `docker/docker-compose-ol-el.yml` if needed).
- Start only EL:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml up -d --build alpen-client
  ```
- Stop:
  ```bash
  docker compose -f docker/docker-compose-ol-el.yml down
  ```
