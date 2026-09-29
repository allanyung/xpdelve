# Keybindings

## Navigation

| Key | Action |
| --- | --- |
| `j`, Down | Select next resource |
| `k`, Up | Select previous resource |
| `PageDown`, `Ctrl+F` | Move down one page |
| `PageUp`, `Ctrl+B` | Move up one page |
| `g`, Home | Select first resource |
| `G`, End | Select last resource |
| `Enter`, `Space` | Toggle the selected subtree |
| Right | Expand the selected subtree or select its first child |
| Left | Collapse the subtree or select its parent |
| `[` | Collapse every subtree |
| `]` | Expand every subtree |
| `z` | Toggle fitted and untruncated table widths |

## Discovery

| Key | Action |
| --- | --- |
| `:` | Open the resource-kind command palette |
| `/` | Filter rows while retaining matching ancestor paths |
| `f` | Find text without hiding rows |
| `n`, `N` | Select the next or previous find match |
| `Esc` | Clear find, text filter, and kind filter |
| `d` | Open captured `kubectl describe` output |
| `y` | Open redacted live YAML |
| `s` | Show the selected resource status |
| `v` | Load related Kubernetes events |
| `c` | Copy the canonical resource identifier with OSC 52 |

## Command Palette And Kind Filtering

Press `:` (`::command` in the main-view legend) to open the command palette. It
lists each unique resource kind in the current trace. Type to fuzzy-filter the
list, use Up/Down to move the selection, and press Enter to show only resources
whose kind exactly matches the selection. Non-matching resources, including
ancestor rows, are hidden.

The selected kind is shown as `Kind: <name>` below the tree and remains active
across trace refreshes.

The `/` text filter supports plain text and field-qualified queries. Plain text
matches the resource identifier or status. Field names and values are
case-insensitive:

| Field | Example |
| --- | --- |
| `kind` | `kind:secret` |
| `group` | `group:example.io` |
| `namespace` | `namespace:default` |
| `status` | `status:waiting` |
| `ready` | `ready:true` |
| `synced` | `synced:unknown` |

The `ready` and `synced` fields accept `true`, `false`, or `unknown`. Text-filter
matches retain their ancestor paths. The kind selected from the command palette
is a separate exact-match filter and does not retain non-matching ancestors.

### Palette Commands

| Command | Action |
| --- | --- |
| `:clear` | Remove the active kind filter; an independent `/` text filter remains active |
| `:skin` | Open the built-in theme picker; applying a theme also saves it to the active configuration file |
| `:quit` | Exit xpdelve through the normal shutdown path |

Esc while the palette is open closes it without changing the current kind.
Esc from the main view clears the kind filter, `/` text filter, and find query
together.

In the theme picker, Up/Down or `j`/`k` changes the selection, Enter applies and
saves it, and Esc or `q` cancels. The active configuration file defaults to
`~/.config/xpdelve/config.toml`.

## Actions And Session

| Key | Action |
| --- | --- |
| `p`, `u` | Pause or unpause the selected Crossplane resource |
| `Ctrl+D` | Open delete confirmation |
| `Ctrl+X` | Select finalizers to remove and confirm |
| `e` | Run `kubectl edit` |
| `r` | Refresh immediately |
| `P` | Pause or resume automatic refresh for this session |
| `?` | Show help |
| `q`, `Ctrl+C` | Quit or cancel the application |

Within delete confirmation, `c` cycles foreground, background, and orphan
propagation. Foreground is always the initial choice.

Content modals use `j`/`k` or Up/Down, PageUp/PageDown, and `g`/`G` for vertical
navigation. Describe and event content wraps long lines. YAML wraps by default;
press `w` to toggle wrapping. `h`/`l` or Left/Right scroll unwrapped YAML
horizontally. `/` starts modal-local search, and `n`/`N` navigate matching
lines. In Describe, live YAML, and Events, drag across text to copy it on mouse
release; visual soft wraps do not add newlines to the copied text. The main
resource table also supports drag-to-copy, while a click without dragging
selects the clicked resource row and a double-click opens its live YAML.
Right-clicking a resource opens a menu for YAML, Edit, Status, Events, and
Describe.
