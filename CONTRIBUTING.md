# Contributing

Thanks for considering a contribution to mouseshare.

## Before you start

This project has one hard constraint that shapes most design decisions:
**stay lightweight**, in both binary size and runtime memory. Before adding
a dependency, ask whether the standard library or the project's existing
crates (`x11rb`, `tokio` with a minimal feature set, etc.) can do it. PRs
that add heavy dependencies for convenience are likely to be asked to find
a lighter path first — see the `[profile.release]` section in the root
`Cargo.toml` and the crate-by-crate breakdown in `README.md`'s "Layout"
section for the existing size/dependency tradeoffs already made.

## Development setup

```bash
git clone https://github.com/PelerYuan/mouseshare.git
cd mouseshare
cargo build --workspace
```

Running the test suite needs a few system tools, since most of this
project's tests exercise real X11 behavior against a throwaway `Xvfb`
display rather than mocking it:

```bash
sudo apt-get install xvfb xdotool xmodmap xclip   # Debian/Ubuntu
cargo test --workspace
```

See the "Running tests" section of `README.md` for which test files need
which tool, and why (mDNS discovery tests additionally need IP multicast
permitted on some interface, including loopback).

Before opening a PR:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All three run in CI on every PR; a red CI check will block merge.

## Making changes

- Keep PRs focused — one logical change per PR is easier to review than a
  bundle of unrelated fixes.
- Match the existing code style: doc comments explain *why*, not *what*;
  no comment at all when the code is already self-explanatory.
- Add or update tests for behavior you change. Most of this codebase is
  tested against real X11 clients/servers rather than mocks (see
  `tests/e2e.rs` and `crates/x11input/tests/`) — follow that pattern
  rather than introducing mocking for its own sake.
- Update `README.md` if you change user-facing behavior, add a crate, or
  change a dependency's feature flags.

## Reporting bugs / requesting features

Use the issue templates — they ask for the minimum needed to act on a
report (OS/desktop environment, mouseshare version, steps to reproduce).

## Platform scope

Only Linux/X11 is implemented today (see `README.md`'s Status section).
Wayland and Windows/macOS support are open, welcome contributions but
substantial undertakings — open an issue to discuss approach before
investing significant time in one. `mouseshare-core` currently calls
`mouseshare-x11input`'s concrete types directly; a second platform would
need that boundary turned into a trait first, which is itself worth
discussing as its own PR before the platform-specific work lands on top
of it.
