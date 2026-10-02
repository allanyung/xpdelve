# Configuration

By default, `xpdelve` reads `~/.config/xpdelve/config.toml`. Use `--config PATH`
to select another file. Command-line arguments override configuration, which
overrides built-in defaults. Unknown fields and invalid or unsupported schema
versions are errors.

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

Placeholders must be separate array entries, as shown in the example; embedded
forms such as `"--context={context}"` are invalid. Set `context_args` or
`namespace_args` to an empty array if a custom trace command does not accept the
corresponding option.

Automatic refresh begins enabled unless `--no-watch` is supplied. Pausing or
resuming it with `P` is session-only and is not stored in configuration.

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
