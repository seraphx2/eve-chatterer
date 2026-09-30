# Contributing to EVE Chatterer

Thanks for taking a look. This is a young, single-maintainer project; issues and
PRs are welcome, and the guidance below is meant to keep things quick rather than
formal. No CLA; the project is MIT.

## Dev setup

You need Windows, [Rust](https://rustup.rs) and [Node.js](https://nodejs.org) 22
or newer. Then:

```sh
cd app
npm install
npm run tauri dev      # Vite on :1430 + the tray app
```

CI runs on every PR (`.github/workflows/ci.yml`) and must be green before merge.
Run the same checks locally first:

```sh
cd app && npm run build                                # svelte-check + vite build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

(The Rust code is hand-formatted, not `rustfmt`-shaped: match the surrounding
style rather than running `cargo fmt`.)

## Before changing behavior

Read [`CLAUDE.md`](CLAUDE.md) (layout, rules and test flags),
[`docs/DESIGN.md`](docs/DESIGN.md) (decisions and why),
[`docs/FINDINGS.md`](docs/FINDINGS.md) (measured facts about EVE's logs,
windows and Windows itself; don't re-derive them) and
[`docs/BACKLOG.md`](docs/BACKLOG.md) (what's planned). For anything
non-trivial, open an issue first so we don't design in opposite directions.

Some rules are firm, because they're what makes the app safe to use:

- Never inject into, read the memory of, or send input to the EVE client.
  Everything comes from the chat log files.
- Never move, change or delete the user's logs.
- Alert windows must never take focus or block clicks.

## Platforms

**Windows only.** EVE Chatterer follows EVE's clients with Windows APIs
(window positions, focus, virtual desktops, notifications), so other
platforms aren't targets. The `core/` crate stays free of UI and keeps its
Windows-only pieces behind `#[cfg(windows)]`.

## Commits & PRs

- Branch from and target `dev`. `main` only receives merges from `dev`, and
  each merge publishes a release.
- Keep commit subjects short and descriptive. Versioning is date-based
  (CalVer), so there's no conventional-commits requirement.
- One logical change per PR where practical.
