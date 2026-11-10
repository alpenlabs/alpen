# Contribution Guidelines

:+1::tada: First off, thanks for taking the time to contribute! :tada::+1:

We accept contributions in the following forms (non-exhaustive):

- **Bug reports**: A good bug report includes a succinct,
  reproducible context and the intended behavior.
- **Feature requests**: Should be followed by a thorough explanation of why
  the feature is important. Please note that not all feature requests will be
  accepted. If in doubt, please consider opening a discussion first.
- **Pull requests**: Clear and correct code with explanatory documentation
  and comments if necessary. If adding new functionality or fixing bugs,
  we expect accompanying tests. Pull requests will undergo a review process,
  and if accepted, the changes will be incorporated into the codebase.
- **Discussions**: Contributions that cannot be framed as bug reports
  or pull requests are good candidates for discussions. Please be polite
  and present civilized arguments in discussions.
- **Security Reports**: If you believe you have found a vulnerability,
  please provide details [here](https://immunefi.com/bug-bounty/alpen-labs/information/) instead.

## Development Tools

Please install the following tools in your development environment to make sure that
you can run the basic CI checks in your local environment:

- [`just`](https://just.systems):
  runs the commands in `.justfile`. `just` lists them.
- [`taplo`](https://taplo.tamasfe.dev/cli/installation/binary.html):
  used to lint and format `TOML` files.
- [`codespell`](https://github.com/codespell-project/codespell):
  used to check for common misspellings in code.
- [`shellcheck`](https://www.shellcheck.net/):
  used to lint shell scripts.
- [`cargo-nextest`](https://nexte.st):
  modern test runner for Rust.
- [`cargo-audit`](https://docs.rs/cargo-audit/latest/cargo_audit/):
  tool to check `Cargo.lock` files for security vulnerabilities.
- [`uv`](https://docs.astral.sh/uv/):
  runs the Python functional tests and their lints.
- [`bitcoind`](https://bitcoincore.org/en/download/):
  must be on `PATH` for the functional tests.
- [`protoc`](https://protobuf.dev/installation/):
  the SP1 crates compile protobuf files, so builds with `--all-features` need it.
  `just lint-check-ws` uses that flag.
- The [SP1 toolchain](https://docs.succinct.xyz/docs/sp1/getting-started/install):
  only needed to build real checkpoint guest artifacts;
  see [`provers/sp1/README.md`](provers/sp1/README.md).

We also provide a [`flake.nix`](flake.nix) with a `devShell` with all the dependencies
necessary for local development.
Install [Nix](https://nixos.org) and run:

```sh
nix develop
```

## Linting and Testing

```sh
just fmt-ws           # format Rust
just lint             # format and lint checks
just test             # unit tests and doctests
just test-functional  # functional tests against a regtest bitcoind
```

`just lint-check-ws` runs clippy alone and fails on warnings, as CI does.
To run one functional test, use `cd functional-tests && ./run_tests.sh -t <test>`;
see [`functional-tests/README.md`](functional-tests/README.md) for more.

## Locally running CI

Before you create a PR, make sure that all the required CI checks pass locally.
For your convenience, a `.justfile` recipe has been created which you can run via:

```bash
just pr
```

`just pr-lite` runs the same checks without the functional tests.

For how the code is organized, see [`docs/`](docs/README.md).
