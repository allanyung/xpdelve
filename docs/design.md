# xpdelve design

## Purpose

`xpdelve` is an opinionated successor to `xpdig`. It preserves Crossplane trace
semantics and familiar workflows while adopting a responsive, asynchronous
Rust architecture and polished modal terminal UI inspired by Sofka.

The application is writable by default and provides `--readonly`. Linux and
macOS on amd64 and arm64 are the initial supported targets.

## Invocation

The canonical invocation is `xpdelve Kind/name`. `xpdelve Kind name` and
`xpdelve Kind.Group/name` are also accepted. There is no `trace` subcommand and
no static stdin or file mode. Reserved commands are `version`, `info`, and
`completion`.

The default source is `crossplane resource trace -o json`. A legacy `--cmd`
string is parsed as shell words but never run through a shell. Structured
configuration separates base, resource, context, and namespace arguments so
optional values cannot leave dangling flags.

## Runtime architecture

One Tokio-driven application loop owns UI state. Terminal input, refresh
timers, trace completion, Kubernetes actions, and external process completion
arrive as typed events over bounded channels. Rendering is synchronous and has
no side effects.

Trace acquisition is asynchronous. JSON parsing, tree projection,
reconciliation, and other CPU work run outside the input/render path. Results
carry generations, allowing stale work to be discarded. The previous complete
tree remains interactive until a validated immutable snapshot can be swapped
in atomically.

Only one trace process runs at a time. Manual refresh cancels the active Unix
process group and starts a new generation. Timer and action refresh requests
coalesce. Output is bounded, stdout and stderr remain separate, and exactly one
complete Crossplane trace JSON document is accepted.

## Domain model

The typed trace envelope contains a generic Kubernetes JSON object, optional
trace error, and recursive children. UI identity is
`group/kind/namespace/name`; UID is revision and safety metadata. Unknown object
fields remain available for YAML, describe, diffing, and future Crossplane
versions.

Projection is depth-first and preserves hierarchy. Selection and expansion are
reconciled by identity across snapshots. UID changes indicate recreation.
Removed nodes are retained briefly, and only current and previous successful
snapshots remain in memory.

Ordinary resource health preserves xpdig's Ready/Synced priority and its
Crossplane v2 missing-condition exception. Package and package-revision roots
retain their Installed/Healthy, image/version, and desired-state semantics.

## UI

The main view is a full-width, initially expanded tree. Ordinary traces display
`OBJECT`, `GROUP`, `SYNCED`, `SYNCED LAST`, `READY`, `READY LAST`, and `STATUS`.
Package traces use their Installed/Healthy schema. `OBJECT` and `GROUP` receive
width priority; transition columns disappear together before either is
truncated. The composition-resource annotation is not a table column.

Describe and YAML use the full terminal. Events and diff modals use nearly the
full terminal; confirmation dialogs remain compact. Content views provide local
search plus vertical and horizontal navigation. YAML, event, detail, and diff
content receive lightweight semantic highlighting. Normal borders use the
active theme's lavender swatch, following Sofka's focused-content role;
destructive confirmation borders use the active theme's red swatch. Color is
the desired default but symbols and text carry the same meaning; `NO_COLOR`,
monochrome, theme customization, and ASCII tree lines are supported. Mouse
capture is not used, preserving normal terminal text selection.

The theme is resolved once before entering the alternate screen and stored in
application state. Renderers request semantic styles instead of embedding RGB
values. Named palettes and swatch overrides follow Sofka's palette model;
xpdelve retains its independent `auto`/`always`/`never` color policy and does not
paint the terminal background.

Find highlights and navigates without hiding rows. Filter limits rows while
retaining ancestor paths. Field-qualified filters initially include kind,
group, namespace, status, ready, and synced.

## Kubernetes operations

Kube-rs discovery resolves dynamic resources. Native APIs provide live YAML,
events, delete, finalizer removal, pause, and unpause. Every mutation re-fetches
and validates the target. `kubectl describe` provides captured content for the
full-screen Describe view. `kubectl edit` remains the permanent edit workflow.

Delete defaults to foreground propagation. Its confirmation modal uses `c` to
cycle Foreground, Background, and Orphan. Finalizer removal defaults to all but
allows individual selection. Successful mutations request an immediate trace
refresh without blocking navigation.

## Security

Kubernetes Secret `data` and `stringData` plus `managedFields` are redacted from
YAML and diff views. Other resources may still contain credential-like values;
broader recursive redaction and warning-gated reveal remain future work. Text
from Kubernetes and external commands is sanitized before terminal rendering.
The application currently has no persistent logging, telemetry, crash upload,
or automatic update checks.

## Configuration and build

Configuration is `~/.config/xpdelve/config.toml`, schema version 1, with
precedence `CLI > config > defaults`. Runtime UI state is not persisted.

Cargo compiles and tests the Rust project. `Taskfile.yml` is the supported
workflow for contributors and CI. The project is Apache-2.0 licensed;
materially adapted code is recorded in `NOTICE` and the affected source files.
