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
| `Enter` | Show the selected resource's problem details, when available |
| Right | Expand the selected subtree or select its first child |
| Left | Collapse the subtree or select its parent |
| `[` | Collapse every subtree below the root, leaving the root expanded |
| `]` | Expand every subtree |
| `h`, Shift+Left | Scroll columns left by four terminal cells |
| `l`, Shift+Right | Scroll columns right by four terminal cells |
| `L` (Shift+L) | Show or hide the logical name column |
| `X` (Shift+X) | Show or hide the external name column |

Expandable rows reserve a fixed disclosure slot immediately before the resource
name. `▾` marks an expanded row and `▸` a collapsed row; ASCII mode uses `-` and
`+`. A leaf's tree branch extends through the same slot, so toggling a row never
shifts resource names or table columns while leaf rows remain visually connected
to the tree.

Enter never expands or collapses a row, and Space has no action in the resource
tree. Use Left/Right or `[`/`]` to control expansion. Enter and Space retain their
existing roles in dialogs, pickers, and text inputs.

Problematic resources display short status labels: `Deleted` for structured
NotFound/404 trace errors, `Error` for other trace or reconciliation/installation
failures, `Creating` for false readiness/health conditions with reason
`Creating`, `Unready` for other false readiness/health conditions, and `Unknown`
or `Warning` for unknown states or those condition reasons. `Deleting` takes
precedence while a resource is being deleted. Reconciliation/installation
failures take precedence over `Creating`. `Deleted` means the referenced object
is not found; it does not require that xpdelve previously observed the object.
Successful statuses remain unchanged, and missing conditions alone are not
errors. Health colors retain their existing meanings.

Press Enter to open a wrapped, scrollable details view containing the full trace
error and any problematic Ready/Synced (or package-specific) conditions. A
condition without a reason or message still reports its state. Details come from
the current trace snapshot, require no API request, and remain fixed while the
view is open, even across refreshes. Esc or `q` closes it. The footer shows
`Enter:details` only when the selected resource has details available, not simply
when it is unhealthy. The `s` key continues to show the resource's raw status YAML.

`OBJECT` stays pinned on the left and `STATUS` stays pinned against the inner
right edge. Only the columns between them and their headers scroll together,
including configured extra columns. When enabled, `LOGICAL NAME` is the first
scrolling column and `EXTERNAL NAME` follows it. They show the
`crossplane.io/composition-resource-name` and `crossplane.io/external-name`
annotations respectively, on any resource kind, or `-` when absent or not a
string. Both columns are hidden by default; `L` (Shift+L) and `X` (Shift+X)
toggle them independently for the current session. Set `[ui]`
`show_logical_name = true` or `show_external_name = true` to start with the
corresponding column visible. Refreshes and configuration reloads retain each
column's visibility unless its configured setting changes. Lowercase
`l` still scrolls right. STATUS text is left-aligned within its
column, with one reserved space before the right border, sized from visible
rows, and capped at 24 terminal cells or one third of
the inner width (with a six-cell minimum on supported terminals). Longer values
are truncated with an ellipsis; the model text remains unchanged. The object
column occupies at most 60% of the space remaining after reserving STATUS, its
right padding, and the dividers; longer names are compacted.

Middle columns can be partially visible at either edge of their viewport. A
horizontal scrollbar appears on the existing bottom border beneath this middle
region when it overflows, with static, centred `▪` squares in the subtle colour,
a dotted track, and arrowheads. The left arrow sits below the gap just after the
OBJECT divider; the right arrow stops before the STATUS divider. An arrow is
dimmed when that end has been reached. Subtle vertical dividers separate both
pinned columns from the scrolling headers and resource cells. ASCII mode
uses `|` for the divider and `<`, `>`, `.`, and `-` for the scrollbar.
Resizing, refreshing, filtering, and collapsing preserve the scroll position,
clamped to the remaining content. Columns are not hidden automatically in narrow
windows; `ui.short` or `--short` can explicitly omit the transition-time columns.
The footer shows `h/l:scroll` at its right edge only when the middle columns
overflow and there is space to scroll them.

Over the tree, a mouse thumbwheel (native horizontal-wheel events) or
Shift+vertical-wheel scrolls the columns in four-cell steps. Terminal support is
required for these mouse events. Ordinary vertical-wheel behavior is unchanged.

## Discovery

| Key | Action |
| --- | --- |
| `:` | Open the resource-kind command palette |
| `/` | Filter rows while retaining matching ancestor paths |
| `f` | Find text without hiding rows |
| `n`, `N` | Select the next or previous find match |
| `Esc` | Clear find, text filter, kind filter, and health filter |
| `d` | Open captured `kubectl describe` output |
| `y` | Open redacted live YAML |
| `s` | Show the selected resource status |
| `E` | Load related Kubernetes events |
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
matches the resource identifier, status label, or problem details. `status:` also
searches problem details, including messages no longer shown in the table.
Field names and values are case-insensitive:

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
| `:health` | Show all resources, only healthy resources, or resources requiring attention |
| `:skin` | Open the built-in theme picker; applying a theme also saves it to the active configuration file |
| `:reload` | Reload the active configuration file, preserving command-line overrides and session state |
| `:quit` | Exit xpdelve through the normal shutdown path |

Esc while the palette is open closes it without changing the current kind.
Esc from the main view clears the kind filter, `/` text filter, find query, and
health filter together; it does not reset excluded kinds.

The exclusion picker identifies entries by API group and kind, with `Usage`
kinds listed first. Checked entries are hidden. Use Up/Down or `j`/`k` to move,
Space to toggle an entry, `a` to show all kinds, `x` to hide all kinds, and `o`
to show only the highlighted kind. Enter applies the staged choices; Esc or `q`
cancels them. Hiding a kind also hides the complete subtree below each matching
resource. Exclusions survive trace refreshes but last only for the current
xpdelve session.

The health picker offers `All`, `Unhealthy`, and `Healthy`. `Unhealthy` includes
warning and unknown states so resources requiring attention are not omitted.
Matching resources retain their ancestor paths for tree context. Health filters
combine with text and kind filters, remain active across trace refreshes, and
are cleared with the other filters by Esc from the main view.

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
lines. In Describe, live YAML, Events, resource details, and mutation errors,
drag across text to copy it on mouse release; visual soft wraps do not add
newlines to the copied text. Hold a drag at the top or bottom edge of a text view
to scroll and extend the selection; wheel scrolling also preserves an active drag.
The main resource table also supports drag-to-copy,
while a click without dragging selects the clicked resource row and a
double-click opens its live YAML.
Tree copying uses the displayed text, including both pinned columns and the visible
portion of horizontally scrolled columns.
Right-clicking a resource opens a menu for YAML, Edit, Status, Events, and
Describe.
