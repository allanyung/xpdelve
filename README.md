# xpdelve

`xpdelve` is a responsive terminal explorer for Crossplane resource traces. It
invokes `crossplane resource trace -o json`, projects the result into an
interactive tree, and keeps the current tree usable while refresh work runs in
the background.

The project is an opinionated Rust successor to
[`xpdig`](https://github.com/brunoluiz/xpdig), with an architecture influenced
by [`sofka`](https://github.com/nklmilojevic/sofka). Portions are adapted from
both projects under Apache-2.0; see [`NOTICE`](NOTICE) for detailed provenance.

![xpdelve demo](docs/assets/xpdelve.gif)

The reproducible nested-Composition environment and VHS recording used for the
project demo are documented in [`demo/`](demo/README.md).

## Status

xpdelve provides live trace browsing, resilient refresh, tree find/filter,
native Kubernetes inspection and mutations, and kubectl-backed Describe and
Edit workflows.

## Installation

### Install script

Install the latest release for the current platform to
`~/.local/bin/xpdelve`:

```console
curl -fsSL https://raw.githubusercontent.com/allanyung/xpdelve/main/install.sh | sh
```

The installer also removes the Gatekeeper quarantine attribute on macOS.

### Manual installation

Download the archive for your platform from the GitHub release:

- `xpdelve_VERSION_Linux_x86_64.tar.gz`
- `xpdelve_VERSION_Linux_arm64.tar.gz`
- `xpdelve_VERSION_Darwin_x86_64.tar.gz`
- `xpdelve_VERSION_Darwin_arm64.tar.gz`

Extract the archive and place `xpdelve` somewhere on your `PATH`:

```console
tar -xzf xpdelve_VERSION_Linux_x86_64.tar.gz
install -m 0755 xpdelve ~/.local/bin/xpdelve
```

Linux archives use the GNU ABI and are built on Ubuntu 22.04. macOS binaries
are currently unsigned and not notarized. Windows and musl builds are not
currently provided.

On macOS, Gatekeeper may prevent the unsigned binary from running. Remove the
quarantine attribute after extracting it:

```console
xattr -d com.apple.quarantine ~/.local/bin/xpdelve
```

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

## Usage

```console
xpdelve ObjectStorage/example
xpdelve --namespace default ObjectStorage example
xpdelve --context development ObjectStorage/example
```

By default, configuration is read from `~/.config/xpdelve/config.toml`; use
`--config PATH` to select another file. See
[`docs/configuration.md`](docs/configuration.md).

### Common options

| Option | Purpose |
| --- | --- |
| `--config PATH` | Read configuration from a different file |
| `--context NAME` | Use a specific Kubernetes context |
| `-n`, `--namespace NAME` | Trace a resource in a specific namespace |
| `--kubeconfig PATH` | Use a specific kubeconfig for trace and Kubernetes operations |
| `--readonly` | Disable mutating actions |
| `--short` | Hide condition transition-time columns |
| `--no-watch` | Start with automatic refresh disabled |
| `--watch-interval SECONDS` | Set the automatic refresh interval |
| `--cmd COMMAND` | Override the trace command prefix without invoking a shell |

Run `xpdelve --help` for the complete CLI reference. Shell completions can be
generated with `xpdelve completion SHELL`.

### Sofka Integration

`xpdelve` can be launched for the selected Kubernetes resource from
[`sofka`](https://github.com/nklmilojevic/sofka). Ensure `xpdelve` is available
on your `PATH`, then add this plugin to your Sofka configuration file:

```toml
[[plugins]]
key = "ctrl-t"
palette = "xpdelve"
name = "Crossplane Delve"
command = "xpdelve"
args = ["-n", "$NAMESPACE", "--context", "$CONTEXT", "$RESOURCE.$GROUP/$NAME"]
mutating = false
output = "terminal"
timeout = "10s"
```

Select a Crossplane resource in Sofka and press `ctrl-t` to open its resource trace
in `xpdelve`. Sofka supplies the selected resource's namespace, kubeconfig
context, resource, API group, and name through the variables in `args`.

## Keys

- `j`/`k` or `Up`/`Down`: move through resources
- `Enter`: show resource problem details when available; the footer hint appears only on those rows
- `Right`: expand or select the first child
- `Left`: collapse or select the parent
- `[`/`]`: collapse or expand the whole tree
- `:`: choose a resource kind; `/`: filter; `f`: find; `n`/`N`: next or previous match
- `d`, `y`, `s`, `E`: describe, live YAML, status, and events
- `p`/`u`: pause or unpause the selected Crossplane resource
- `ctrl+d`: delete with a propagation-policy confirmation
- `ctrl+x`: selectively remove finalizers after confirmation
- `e`: run `kubectl edit`
- `c`: copy the canonical resource identifier using OSC 52
- `h`/`l`, Shift+Left/Right: scroll middle columns while resource names and status stay pinned; thumbwheel or Shift+wheel also works over the tree
- Mouse wheel: move up/down through the tree, or scroll text views and help vertically (three rows per wheel event)
- `r`: refresh; `P`: pause automatic refresh; `?`: help; `q`: quit

### Extra tree columns

Configure extra columns for any resource kind/API group in
`~/.config/xpdelve/config.toml`:

```toml
[[extra_columns."Bucket.s3.aws.upbound.io"]]
name = "REGION"
path = "/spec/forProvider/region"
```

Definitions apply to all matching resources. The main tree shows the union of
configured columns for visible kinds, immediately before STATUS; unrelated rows
show `-`. Paths use JSON Pointer syntax, and an optional `width` fixes a column's
width. See [extra-column configuration](docs/configuration.md#extra-columns)
for matching, shared headers, and display rules.

### Command palette, filtering, and exclusion

Press `:`—shown as `::command` in the main-view legend—to open the command
palette. It lists the unique resource kinds in the current trace. Type to
fuzzy-filter the list, use Up/Down to select a kind, and press Enter to show only
resources of that exact kind. Non-matching resources and ancestors are hidden,
and the kind selection remains active across trace refreshes. Duplicate kind
names from different API groups appear as separate `Kind.group` entries and are
filtered independently.

To restore the full tree, open the palette, type `clear`, and select the
`:clear` entry. `:clear` removes only the selected kind, so an independent
`/` text filter remains active. Esc closes an open palette without changing the
selection; Esc from the main view clears the kind, text, and find filters
together without changing excluded kinds.

Enter `:exclude` to hide resource kinds from the tree for the current session.
The picker distinguishes kinds by API group and lists `Usage` kinds first.
Checked entries are hidden, along with their complete subtrees. Use Space to
toggle an entry, `a` to show all, `x` to hide all, or `o` to show only the
highlighted kind, then press Enter to apply. Exclusions remain active across
trace refreshes but are not written to configuration.

Enter `:skin` to open the theme picker. Use Up/Down to choose a built-in theme
and Enter to apply it. The choice takes effect immediately and is saved as
`skin.name` in the active configuration file (normally
`~/.config/xpdelve/config.toml`).

Enter `:quit` to close xpdelve from the command palette.

Enter `:reload` to reload the active configuration file without restarting.
Command-line overrides and session filters, selection, and pause state are
preserved. Invalid configuration leaves the current settings unchanged.

Enter `:health` to filter resources by health status. Choose `All` to show every
resource, `Unhealthy` to show only resources requiring attention (including
warning and unknown states), or `Healthy` to show only resources without problems.
Matching resources retain their ancestor paths for tree context. The health filter
combines with text and kind filters and remains active across trace refreshes.
Press Esc from the main view to clear the health filter along with kind, text,
and find filters.

See [the full keybinding and filtering guide](docs/keys.md) for palette controls
and the distinction between kind filtering, text filtering, and find.

## Deletion and trace recovery

Delete confirmation defaults to foreground propagation. Press `c` in the
confirmation to cycle through foreground, background, and orphan behavior.

If the root resource can no longer be found, xpdelve clears the stale trace and
stops automatic polling. Press `r` to retry manually; successful recovery
restores the tree and normal refresh behavior.

## Mouse Support

On the main resource table, click a row to select it, double-click to open its
live YAML, or drag across displayed text to highlight and copy it on release.
Right-click a row for YAML, Edit, Status, Events, and Describe actions. Describe,
live YAML, Events, Error, and SmallError views offer the same drag-to-copy behavior;
visual soft wraps are not included as newlines.
Describe, YAML, and Events use the full terminal and wrap long lines. Content
views support `/` search and vertical scrolling. In YAML, `w` toggles wrapping;
`h`/`l` or Left/Right scroll unwrapped YAML horizontally.

## Themes

xpdelve supports the same built-in theme catalog as Sofka. Omit `skin.name` to
select Catppuccin Latte or Mocha based on the detected terminal background; if
detection fails, xpdelve defaults to Mocha. You can also configure a named theme
and optional swatch overrides, or select and persist a theme interactively with
`:skin`.

See [`docs/configuration.md`](docs/configuration.md#themes) for available themes
and configuration options.

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

To prepare a release from a clean `main` branch, provide its version without a
`v` prefix:

```console
task release VERSION=0.1.1
```

The task updates Cargo's package version and lockfile, runs the local CI gate,
then creates a release commit and annotated tag. It prints the command for
pushing the commit and tag; it does not push them automatically.

## License

Licensed under Apache License, Version 2.0. See `LICENSE` and `NOTICE`.
