# Configuration

By default, `xpdelve` reads `~/.config/xpdelve/config.toml`. Use `--config PATH`
to select another file. Command-line arguments override configuration, which
overrides built-in defaults. Unknown fields and invalid or unsupported schema
versions are errors.

Use `:reload` in the command palette to reload the active configuration file.
Command-line arguments still take precedence. UI settings, themes, extra columns,
and the automatic refresh interval update immediately; trace settings apply to
the next trace run (press `r` to refresh now). Session filters, selection, and
pause state are preserved. If the file is missing or invalid, a small error modal
is shown and the current configuration remains active. Use Up/Down or `j`/`k` to
scroll the error and Esc or `q` to close it.

```toml
schema_version = 1
read_only = false

[trace]
program = "crossplane"
args = ["resource", "trace", "-o", "json"]
resource_args = ["{resource}"]
context_args = ["--context", "{context}"]
namespace_args = ["--namespace", "{namespace}"]
interval_seconds = 5
# timeout_seconds = 60
stdout_limit_bytes = 268435456
stderr_limit_bytes = 1048576
retry_backoff_max_seconds = 60

[ui]
color = "auto"
ascii = false
short = false

[skin]
# Omit name to detect a light/dark terminal and select Latte/Mocha.
name = "catppuccin-mocha"

[skin.colors]
# Optional six-digit RGB overrides, with or without '#'.
# lavender = "#b4befe"
# red = "#f38ba8"
```

## Settings

| Setting | Purpose |
| --- | --- |
| `schema_version` | Configuration schema; currently must be `1` |
| `read_only` | Disable mutating actions when `true` |
| `trace.program` | Trace executable; defaults to `crossplane` |
| `trace.args` | Arguments placed immediately after the trace executable |
| `trace.resource_args` | Arguments containing exactly one standalone `{resource}` placeholder |
| `trace.context_args` | Arguments appended when `--context` is set; a non-empty list must contain exactly one standalone `{context}` placeholder |
| `trace.namespace_args` | Arguments appended when `--namespace` is set; a non-empty list must contain exactly one standalone `{namespace}` placeholder |
| `trace.interval_seconds` | Automatic refresh interval; must be at least one second |
| `trace.timeout_seconds` | Optional timeout for each trace process; omit it for no timeout |
| `trace.stdout_limit_bytes` | Maximum captured trace stdout size |
| `trace.stderr_limit_bytes` | Maximum captured trace stderr size |
| `trace.retry_backoff_max_seconds` | Maximum delay between retries after repeated failures |
| `ui.color` | Color policy: `auto`, `always`, or `never` |
| `ui.ascii` | Use ASCII rather than Unicode tree lines when `true` |
| `ui.short` | Hide condition transition-time columns when `true` |
| `skin.name` | Built-in theme name; omit it for terminal-background detection |
| `skin.colors` | Optional per-swatch RGB overrides |
| `extra_columns` | Additional main-tree columns keyed by exact resource kind and API group |

Placeholders must be separate array entries, as shown in the example; embedded
forms such as `"--context={context}"` are invalid. Set `context_args` or
`namespace_args` to an empty array if a custom trace command does not accept the
corresponding option.

Automatic refresh begins enabled unless `--no-watch` is supplied. Pausing or
resuming it with `P` is session-only and is not stored in configuration.

## Extra columns

Add columns to the existing main resource tree with `[[extra_columns."Kind.group"]]`.
These definitions do not create new views or replace built-in columns. Every
extra column appears immediately before STATUS, including with `--short` and in
package traces.

```toml
[[extra_columns."Bucket.s3.aws.upbound.io"]]
name = "REGION"
path = "/spec/forProvider/region"
width = 14

[[extra_columns.Secret]]
name = "OWNER"
path = "/metadata/annotations/example.com~1owner"
```

Keys match the object's exact, case-sensitive **kind and API group**, not its
plural resource name or the CLI argument. Use a bare kind such as `Secret` for
the core API group. A definition applies to every matching resource, regardless
of API version, name, or namespace. Wildcards and per-instance overrides are not
supported.

### Fields and display

- `name` is the column header. Headers are trimmed and displayed in uppercase;
  comparison is case-insensitive.
- `path` is a non-empty JSON Pointer (RFC 6901) into the resource's trace JSON,
  such as `/status/atProvider/location` or `/spec/ports/0/port`. Escape `/` in a field
  name as `~1`, and `~` as `~0`. Dot notation and JSONPath expressions are not
  supported.
- `width` is optional. Omit it to size the column from its header and currently
  visible values. Set it to a display-cell width from 1 to 65535 for fixed-width
  truncation. Columns remain reachable through horizontal scrolling while
  OBJECT stays pinned on the left and STATUS stays pinned on the right.

Strings, numbers, and booleans render as text. Missing fields, nulls, objects,
and arrays show `-`; select individual fields or array elements to display them.
Terminal control sequences are made inert and newlines/tabs are flattened to
spaces. Core Secret `data` and `stringData` payloads show `<redacted>` instead
of their contents. Other resources may contain sensitive values; choose paths
accordingly.

Values come from the current trace snapshot and update on trace refresh, without
additional Kubernetes requests. Use `:reload` after editing configuration.
Extra values are not included in text filtering or find navigation.

### Mixed-resource trees

The table shows the union of extra columns configured for kinds in the currently
visible tree. Kind/text/health filtering, exclusions, and collapse can change
this union; vertical scrolling does not. A row without a matching definition
for a column shows `-`.

Resource keys are processed in lexical order, then columns in declaration order.
Headers shared across kinds produce a single column at their first occurrence;
each kind may use a different path. If any matching definition supplies a fixed
width for a shared header, the largest supplied width wins. Otherwise the shared
column is sized automatically.

Invalid resource keys, empty/control-character headers, collisions with built-in
headers, duplicate headers within a kind, malformed pointers, invalid widths,
and unknown fields are configuration errors. Existing configuration files need
no changes, and `schema_version` remains `1`.

## Themes

`skin.name` accepts:

- `catppuccin-mocha`, `catppuccin-latte`, `catppuccin-frappe`, `catppuccin-macchiato`
- `gruvbox-dark`, `gruvbox-light`
- `nord`, `dracula`
- `solarized-dark`, `solarized-light`
- `tokyo-night`, `one-dark`
- `rose-pine`, `rose-pine-dawn`
- `monokai`
- `flexoki-dark`, `flexoki-light`

If `skin.name` is omitted, xpdelve performs a best-effort terminal background
query before entering the alternate screen. Light terminals select Catppuccin
Latte; dark or undetectable terminals select Catppuccin Mocha.

The same themes can be selected interactively with `:skin`. Choosing a theme
applies it immediately and writes its name to `skin.name` in the active
configuration file. The default is `~/.config/xpdelve/config.toml`; when
`--config` is used, that file is updated instead. Existing settings, comments,
and `skin.colors` overrides are retained.

`skin.colors` can override these palette swatches: `rosewater`, `flamingo`,
`pink`, `mauve`, `red`, `maroon`, `peach`, `yellow`, `green`, `teal`, `sky`,
`sapphire`, `blue`, `lavender`, `text`, `subtext1`, `subtext0`, `overlay1`,
`overlay0`, `surface2`, `surface1`, `surface0`, `base`, `mantle`, and `crust`.
Unknown theme names or swatches and malformed colors are configuration errors.

`ui.color` controls whether the resolved palette is emitted. `auto` respects
`NO_COLOR`, `always` overrides it, and `never` uses monochrome styles. Themes do
not paint the terminal background.
