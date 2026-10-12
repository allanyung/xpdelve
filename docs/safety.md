# Safety

`xpdelve` permits writes by default to preserve the intended interactive
workflow. Use `--readonly` or `read_only = true` to disable mutation bindings.

## Mutation Checks

- Every native mutation re-fetches the selected resource.
- The UID from the trace must match the live UID.
- Delete sends UID and resource-version preconditions to the API server.
- Annotation and finalizer patches atomically test UID and resourceVersion.
- A conflict is reported instead of silently overwriting concurrent changes.
- Only one mutation per resource and four mutations globally may run at once. A bulk delete or pause counts as one running action per target and is rejected if any target is busy, or if four are already running.
- Bulk actions apply every check above to each resource independently, run sequentially, and report all failures. Marks are tied to UID, so a recreated resource is never targeted by an earlier mark. Read-only mode blocks all bulk mutations.

Mutation failures open in a large, wrapped, scrollable error view that remains
visible across trace refreshes until dismissed. The full message can be searched
or copied instead of being truncated to the status line.

Delete requires confirmation and defaults to foreground propagation. Press `c`
to cycle to background or orphan propagation. The application submits the
request and follows progress through subsequent traces rather than blocking
until Kubernetes garbage collection finishes.

Finalizer removal starts with nothing selected, requires choosing finalizer names,
and then shows a per-resource review of exactly what is removed and kept. When the
change is applied, each resource's live finalizers must still equal the reviewed
list; otherwise that resource fails and must be reviewed again, so a finalizer
added after the review is never removed unseen. Removing finalizers can bypass
controller cleanup and leave external resources behind.

## Sensitive Data

Secret `data` and `stringData` are redacted from the live YAML modal, and
`managedFields` is omitted. Other resource kinds are not recursively redacted,
so their YAML or Describe output may contain credential-like values. Text from
the cluster and external commands is sanitized to prevent terminal control
sequences from executing. The trace command and kubectl still run with the
user's normal environment and credentials; avoid placing credentials directly
in custom command arguments.
