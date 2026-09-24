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
| `Enter`, `Space`, Right | Toggle the selected subtree |
| Left | Collapse the subtree or select its parent |
| `[` | Collapse every subtree |
| `]` | Expand every subtree |
| `z` | Toggle fitted and untruncated table widths |

## Discovery

| Key | Action |
| --- | --- |
| `/` | Filter rows while retaining matching ancestor paths |
| `f` | Find text without hiding rows |
| `n`, `N` | Select the next or previous find match |
| `Esc` | Clear find and filter |
| `d` | Open captured `kubectl describe` output |
| `y` | Open redacted live YAML |
| `s` | Show the selected resource status |
| `v` | Load related Kubernetes events |
| `c` | Copy the canonical resource identifier with OSC 52 |

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
release; visual soft wraps do not add newlines to the copied text.
