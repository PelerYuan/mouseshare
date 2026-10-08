# Contributing to mouseshare

[简体中文](CONTRIBUTING.zh-CN.md)

Thanks for considering a contribution! Bug reports, translations, docs fixes
and code are all valuable, and small contributions are welcome. By taking part
you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- **Report a bug** or **suggest a feature** with the
  [issue forms](https://github.com/PelerYuan/mouseshare/issues/new/choose).
  Search existing issues first.
- **Ask a question** in
  [Discussions](https://github.com/PelerYuan/mouseshare/discussions) (see
  [SUPPORT.md](SUPPORT.md)).
- **Improve the docs or translate.** Every document has an English original and
  a `*.zh-CN.md` counterpart. UI strings live in `crates/gui/src/strings.rs`.
- **Pick up an issue** labelled
  [`good first issue`](https://github.com/PelerYuan/mouseshare/labels/good%20first%20issue)
  or [`help wanted`](https://github.com/PelerYuan/mouseshare/labels/help%20wanted),
  and say you are working on it.
- **Report a security problem** privately — see [SECURITY.md](SECURITY.md).

For anything larger than a small fix, please open an issue first so we can agree
on the approach before you invest time.

## Development setup

You need a recent stable Rust (1.88+) and, for the tests, a few X11 tools:

```bash
sudo apt-get install xvfb xdotool x11-xserver-utils xclip   # Debian/Ubuntu
git clone https://github.com/PelerYuan/mouseshare.git
cd mouseshare
cargo build --workspace
cargo test --workspace
```

Most tests exercise real X11 behaviour against throwaway `Xvfb` displays rather
than mocks. Which test needs which tool is noted at the top of each test file.
The mDNS tests additionally need IP multicast allowed on some interface
(including loopback).

Run the GUI from source with `cargo run -p mouseshare-gui`, and the CLI with
`cargo run -p mouseshare -- --help`.

To build release packages locally: `packaging/build.sh` (outputs to `dist/`).

## Project layout

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). In short: `protocol` and
`layout` are pure logic, `net` is the secure transport, `x11input` is the only
crate that talks to X11, `core` is the runtime shared by the CLI and the GUI,
`config` persists settings, and `gui` is the egui application.

## Pull request workflow

1. **Fork** the repository (or create a branch if you have write access) and
   branch from `master`: `git switch -c fix/short-description`.
2. Make focused changes — one logical change per pull request.
3. Before pushing, run:
   ```bash
   cargo fmt --all
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```
4. Add or update **tests** for behaviour you change, and update the **docs**
   (English *and* the `.zh-CN.md` counterpart — if you cannot write Chinese, say
   so in the PR and we will help) and the `[Unreleased]` section of
   `CHANGELOG.md` for user-visible changes.
5. Open a pull request using the template. Describe *what* and *why*, and link
   the issue (`Fixes #123`).
6. CI must be green and a maintainer must approve. Review comments are a
   conversation, not a verdict; push follow-up commits to the same branch.
7. Pull requests are **squash-merged**, so your commit history on the branch
   does not need to be tidy, but the PR title should be.

`master` is protected: changes land only through pull requests with passing
checks.

### Commit and PR titles

Use a short imperative summary, optionally prefixed with the area:
`gui: fix canvas snapping across monitors`, `net: reject oversized handshake`,
`docs: clarify firewall setup`.

### Code style

- `cargo fmt` and clippy-clean (`-D warnings`).
- Match the surrounding code. Comments explain *why*, not *what*; no comment
  when the code is self-explanatory.
- Keep it **lightweight**. Before adding a dependency, check whether the
  standard library or an existing dependency can do the job; disable default
  features where possible. PRs that add heavy dependencies for convenience will
  be asked to find a lighter path.
- No `unsafe` outside well-justified, commented spots.
- User-visible strings go through `strings.rs` with both English and Chinese
  entries (a unit test fails if a key is missing).

## Adding a translation

1. Copy the English strings in `crates/gui/src/strings.rs`, add your language
   code to `Lang`/`Language`, and translate every entry.
2. Translate the docs as `*.<lang>.md` next to the originals and link them from
   the language switcher line at the top of each file.
3. Open a PR — partial translations are welcome as drafts.

## Platform scope

Only Linux/X11 is implemented today. Wayland, Windows and macOS support are
welcome but substantial; see the [roadmap](docs/ROADMAP.md) and open an issue to
discuss the approach before investing significant time. The seam for a second
backend is the `x11input` crate, which `core` currently uses through concrete
types; turning that into a trait is a worthwhile first PR on its own.

## For maintainers: releasing

1. Move the `[Unreleased]` changelog entries under a new version heading (in
   both languages) and bump `version` in `Cargo.toml` and each `crates/*/Cargo.toml`.
2. Merge that through a pull request.
3. Tag the merge commit `vX.Y.Z` and push the tag. The *Release* workflow builds
   the tarball and `.deb`, writes `SHA256SUMS`, and publishes a GitHub Release
   using the changelog section.
