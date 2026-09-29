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

Expandable rows reserve a fixed disclosure slot immediately before the resource
name. `▾` marks an expanded row and `▸` a collapsed row; ASCII mode uses `-` and
`+`. Leaves keep the same slot blank, so toggling a row never shifts resource
names or table columns.

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

## Command Palette, Kind Filtering, And Exclusion

Press `:` (`::command` in the main-view legend) to open the command palette. It
lists each unique resource kind in the current trace. Type to fuzzy-filter the
list, use Up/Down to move the selection, and press Enter to show only resources
whose kind exactly matches the selection. Non-matching resources, including
ancestor rows, are hidden. If the same kind occurs in more than one API group,
each entry is qualified as `Kind.group` and filters only that group.

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
| `:exclude` | Choose resource kinds to hide for the current session |
| `:skin` | Open the built-in theme picker; applying a theme also saves it to the active configuration file |
| `:quit` | Exit xpdelve through the normal shutdown path |

Esc while the palette is open closes it without changing the current kind.
Esc from the main view clears the kind filter, `/` text filter, and find query
together; it does not reset excluded kinds.

The exclusion picker identifies entries by API group and kind, with `Usage`
kinds listed first. Checked entries are hidden. Use Up/Down or `j`/`k` to move,
Space to toggle an entry, `a` to show all kinds, `x` to hide all kinds, and `o`
to show only the highlighted kind. Enter applies the staged choices; Esc or `q`
cancels them. Hiding a kind also hides the complete subtree below each matching
resource. Exclusions survive trace refreshes but last only for the current
xpdelve session.

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

If a delete, pause, unpause, or finalizer-removal operation fails, xpdelve opens
a large, persistent error view instead of placing the full message in the status
line. The view remains open across trace refreshes until Esc or `q` closes it.

Content modals use `j`/`k` or Up/Down, PageUp/PageDown, and `g`/`G` for vertical
navigation. Describe, event, and error content wraps long lines. YAML wraps by
default; press `w` to toggle wrapping. `h`/`l` or Left/Right scroll unwrapped
YAML horizontally. `/` starts modal-local search, and `n`/`N` navigate matching
lines. In Describe, live YAML, Events, and mutation errors, drag across text to
copy it on mouse release; visual soft wraps do not add newlines to the copied
text. The main resource table also supports drag-to-copy, while a click without
dragging selects the clicked resource row and a double-click opens its live YAML.
Right-clicking a resource opens a menu for YAML, Edit, Status, Events, and
Describe.
