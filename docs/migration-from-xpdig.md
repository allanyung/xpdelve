# Migrating from xpdig

The canonical invocation changes from `xpdig trace Kind/name` to
`xpdelve Kind/name`. `Kind name` remains accepted. Static stdin mode and k9s
integration are intentionally not provided.

Existing `--context`, `--namespace`, `--short`, `--no-watch`,
`--watch-interval`, and `--cmd` workflows remain available. `--cmd` now uses
shell-word parsing but does not invoke a shell.

Notable behavior changes:

- Trace stdout and stderr are kept separate, and failed commands exit or retry
  with a nonzero-status diagnostic.
- Refresh failures retain the previous interactive tree and retry.
- Tree branches can be collapsed, and find and filter are separate operations.
- `Ctrl+D` is delete only; page navigation uses PageDown or `Ctrl+F`.
- Delete defaults to foreground cascading and permits changing propagation in
  the confirmation modal.
- Kubernetes inspection and mutations use native APIs, except for permanent
  `kubectl edit` and captured `kubectl describe` integration.
- Mutations validate live UID/resourceVersion data before acting.
- Stdin trace input is not supported.
