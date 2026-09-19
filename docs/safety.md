# Safety

`xpdelve` permits writes by default to preserve the intended interactive
workflow. Use `--readonly` or `read_only = true` to disable mutation bindings.

## Mutation Checks

- Every native mutation re-fetches the selected resource.
- The UID from the trace must match the live UID.
- Delete sends UID and resource-version preconditions to the API server.
- Annotation and finalizer patches atomically test UID and resourceVersion.
- A conflict is reported instead of silently overwriting concurrent changes.
- Only one mutation per resource and four mutations globally may run at once.

Delete requires confirmation and defaults to foreground propagation. Press `c`
to cycle to background or orphan propagation. The application submits the
request and follows progress through subsequent traces rather than blocking
until Kubernetes garbage collection finishes.

Finalizer removal requires a separate confirmation and permits individual
finalizers to be deselected. Removing finalizers can bypass controller cleanup
and leave external resources behind.

## Sensitive Data

Secret `data` and `stringData` are redacted from the live YAML modal, and
`managedFields` is omitted. Other resource kinds are not recursively redacted,
so their YAML or Describe output may contain credential-like values. Text from
the cluster and external commands is sanitized to prevent terminal control
sequences from executing. The trace command and kubectl still run with the
user's normal environment and credentials; avoid placing credentials directly
in custom command arguments.
