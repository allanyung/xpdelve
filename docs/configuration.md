# Configuration

`xpdelve` reads `~/.config/xpdelve/config.toml`. Command-line arguments override
configuration, which overrides built-in defaults. Unknown fields and invalid or
unsupported schema versions are errors.

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
horizontal_scroll = false
short = false

[skin]
# Omit name to detect a light/dark terminal and select Latte/Mocha.
name = "catppuccin-mocha"

[skin.colors]
# Optional six-digit RGB overrides, with or without '#'.
# lavender = "#b4befe"
# red = "#f38ba8"
```

Automatic refresh always begins enabled. Pausing refresh is session-only and is
not stored in configuration.

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

`skin.colors` can override these palette swatches: `rosewater`, `flamingo`,
`pink`, `mauve`, `red`, `maroon`, `peach`, `yellow`, `green`, `teal`, `sky`,
`sapphire`, `blue`, `lavender`, `text`, `subtext1`, `subtext0`, `overlay1`,
`overlay0`, `surface2`, `surface1`, `surface0`, `base`, `mantle`, and `crust`.
Unknown theme names or swatches and malformed colors are configuration errors.

`ui.color` controls whether the resolved palette is emitted. `auto` respects
`NO_COLOR`, `always` overrides it, and `never` uses monochrome styles. Themes do
not paint the terminal background.
