# xpdelve

`xpdelve` is a responsive terminal explorer for Crossplane resource traces. It
invokes `crossplane resource trace -o json`, projects the result into an
interactive tree, and keeps the current tree usable while refresh work runs in
the background.

The project is an opinionated Rust successor to
[`xpdig`](https://github.com/brunoluiz/xpdig), with an architecture influenced
by [`sofka`](https://github.com/nklmilojevic/sofka). Portions are adapted from
both projects under Apache-2.0; see [`NOTICE`](NOTICE) for detailed provenance.

## Status

Version `0.1.0` provides live trace browsing,
resilient refresh, tree find/filter, native Kubernetes inspection and
mutations, and kubectl-backed Describe and Edit workflows. See
[`docs/implementation.md`](docs/implementation.md) for remaining release work.
See [`CHANGELOG.md`](CHANGELOG.md) for release history.

## Installation

Download the archive for your platform from the GitHub release:

- `xpdelve_0.1.0_Linux_x86_64.tar.gz`
- `xpdelve_0.1.0_Linux_arm64.tar.gz`
- `xpdelve_0.1.0_Darwin_x86_64.tar.gz`
- `xpdelve_0.1.0_Darwin_arm64.tar.gz`

Extract the archive and place `xpdelve` somewhere on your `PATH`:

```console
tar -xzf xpdelve_0.1.0_Linux_x86_64.tar.gz
install -m 0755 xpdelve_0.1.0_Linux_x86_64/xpdelve ~/.local/bin/xpdelve
```

Linux archives use the GNU ABI and are built on Ubuntu 22.04. macOS binaries
are currently unsigned and not notarized. Windows and musl builds are not
currently provided.

## Prerequisites

### Runtime

- Linux or macOS
- A Crossplane CLI version that provides `crossplane resource trace -o json`
- `kubectl` for Describe and Edit actions
- Access to a Kubernetes cluster through the normal kubeconfig environment

### Building From Source

- Rust 1.98.1
- A C compiler, linker, CMake, and platform development tools
- [go-task](https://taskfile.dev/) for the recommended contributor workflow

go-task is optional; every task ultimately invokes Cargo and the equivalent
Cargo commands can be run directly.

## Usage

```console
xpdelve ObjectStorage/example
xpdelve --namespace default ObjectStorage example
xpdelve --context development ObjectStorage/example
```

Configuration is read from `~/.config/xpdelve/config.toml`. See
[`docs/configuration.md`](docs/configuration.md).

## Keys

- `j`/`k` or arrows: move through resources
- `Enter`, `Space`, `Right`, `Left`: expand or collapse the tree
- `[`/`]`: collapse or expand the whole tree
- `/`: filter; `f`: find; `n`/`N`: next or previous match
- `d`, `y`, `v`, `i`: describe, live YAML, events, and previous-trace diff
- `p`/`u`: pause or unpause the selected Crossplane resource
- `Ctrl+D`: delete with a propagation-policy confirmation
- `Ctrl+X`: selectively remove finalizers after confirmation
- `e`: run `kubectl edit`
- `c`: copy the canonical resource identifier using OSC 52
- `z`: toggle fitted/full-width columns; `Alt+h`/`Alt+l`: horizontal scroll
- `r`: refresh; `P`: pause automatic refresh; `?`: help; `q`: quit

Delete confirmation defaults to foreground propagation. Press `c` in the
confirmation to cycle through foreground, background, and orphan behavior.

The application does not capture mouse events. Text selection therefore uses
the terminal emulator's normal behavior, which may require a modifier key.
Describe and YAML use the full terminal; events and diff views use nearly the
full terminal. Content views support `/` search plus vertical and horizontal
scrolling. All normal borders use Sofka's focused-content Catppuccin lavender
accent; destructive confirmation borders remain red.

If the root resource can no longer be found, xpdelve clears the stale trace and
stops automatic polling. Press `r` to retry manually; successful recovery
restores the tree and normal refresh behavior.

xpdelve supports the same built-in theme catalog as Sofka. Omit `skin.name` to
select Catppuccin Latte or Mocha from the terminal background, or configure a
named theme and optional swatch overrides. See
[`docs/configuration.md`](docs/configuration.md#themes).

## Safety

Native mutations re-fetch the selected object and verify its UID. Deletes also
use UID and resource-version server preconditions; patches test UID and
resourceVersion atomically. Secret payloads are redacted in YAML views, and
untrusted cluster text is sanitized before terminal rendering. Non-Secret
resources may still contain credential-like values. Use `--readonly` to disable
mutations.

## Development

```console
task build
task test
task lint
task check
task run -- ObjectStorage/example
```

Task is the supported contributor interface. Cargo remains the underlying Rust
build system and can also be used directly.

## License

Licensed under Apache License, Version 2.0. See `LICENSE` and `NOTICE`.
