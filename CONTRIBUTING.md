# Contributing

Thanks for helping make Indonesian market data easier to work with.

## Setup

```sh
git clone https://github.com/fzn0x/idx.git
cd idx
cargo build
cargo test
```

Rust stable (1.89+) is the only requirement. `rust-toolchain.toml` pins
`rustfmt` and `clippy` as components.

With Nix, `nix develop` gives that toolchain plus rust-analyzer and Node 22
(for `web/` and `npm/`); the commands are the same. `nix flake check` runs fmt,
clippy, the tests, a `cargo check` of every target with the declared minimum
Rust (Cargo.toml `rust-version`) and a smoke test of the packaged binary in a
sandbox.

## Before you open a pull request

```sh
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three on Linux, macOS and Windows.

## Ground rules

- **Tests never hit the network.** Record a real response into
  `tests/fixtures/<provider>/` and test the parser against it. Use
  `http::FixtureBackend` for request-flow tests. Live checks belong in
  `scripts/smoke.sh`.
- **Never invent data.** A field the source did not return is `null`.
- **JSON output is a contract.** See `docs/json-output.md` before changing a
  field name.
- **Conventional Commits.** `feat:`, `fix:`, `docs:`, `test:`, `refactor:`,
  `chore:`, `ci:`. Scope is optional: `feat(yahoo): ...`.

## Adding a provider or command

`AGENTS.md` has the step-by-step guide. It is written for AI agents but
applies to people too.

## Reporting a broken data source

Free endpoints change without notice. If a command starts failing, open an
issue with the output of:

```sh
idx -vv --no-cache stocks quote BBCA 2>&1
```

Remove anything you consider private before pasting. The `-vv` output includes
the URLs requested and HTTP status codes, which is usually enough to diagnose.

## Code of conduct

See `CODE_OF_CONDUCT.md`.
