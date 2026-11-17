# diffio

A read-only terminal Git diff viewer for watching changes made by an agent, editor,
or another process. It refreshes every 500 ms, keeps your selected file and scroll
position, and removes committed changes from the file tree.

## Run

Requires Git and Rust 1.95.0 (selected by `rust-toolchain.toml`).

```sh
cargo run -- /path/to/repository
# Or install the executable:
cargo install --locked --path .
diffio
```

The optional path may point to a repository, a subdirectory, or a linked worktree.
The current directory is used when omitted. Run in an interactive terminal;
`diffio --help` and `diffio --version` also work without one.

## Reading changes

The left pane contains a collapsible tree of changed files, including untracked
files and excluding ignored files. Its two status columns are Git's **X** (index)
and **Y** (working tree): `A` added, `M` modified, `D` deleted, `R` renamed,
`??` untracked, and `U` or an unmerged pair for conflicts. `·` means unchanged.

- **All** (`1`): HEAD versus current working content, including untracked files.
  Staged entries remain visible even if their working content matches HEAD; the
  diff explains when there is no net change.
- **Staged** (`2`): HEAD versus the index.
- **Unstaged** (`3`): index versus the working tree, including untracked files.

Repositories without a first commit use an empty baseline. Binary files and
conflicts get explicit summaries. Renames and mode changes keep their Git metadata.
Selecting a file that later becomes clean leaves a message in the diff pane,
without moving you to another file.

Auto layout uses side-by-side diffs when the diff pane is at least 104 columns
wide, otherwise unified diffs. At widths below 60 columns, only the focused pane
is shown; Tab switches between them. Changed lines have red/green backgrounds,
with stronger color on changed characters. Long lines scroll horizontally.

## Keys

| Key | Action |
| --- | --- |
| Arrows / `hjkl` | Navigate directories and files, or scroll the diff |
| Enter | Toggle a directory or open a file in the diff pane |
| Tab | Switch pane focus |
| `1` / `2` / `3` | All / Staged / Unstaged |
| `[` / `]` | Previous / next hunk |
| `/` | Search filenames or diff text in the focused pane |
| Enter / Escape during search | Submit / cancel; typing is captured until then |
| `n` / `N` | Next / previous search match |
| `v` | Cycle auto / side-by-side / unified layout |
| `t` | Hide / show the tree |
| Page Up / Page Down | Scroll a page |
| Home / End or `g` / `G` | First / last row |
| Mouse wheel | Scroll the pane under the pointer |
| Mouse click | Select a file or toggle a directory |
| `r` | Refresh immediately |
| `?` | Show help |
| `q` | Quit (or close help) |

The main bindings do not require Ctrl or Alt. In Zellij, use normal or locked mode
so its pane/tab/scroll modes do not consume navigation keys. Mouse reporting is
enabled while diffio is running and disabled on exit. The terminal is restored
on normal exit, returned errors, and Rust panics.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
```

One background worker runs Git status and reloads the selected diff on every
refresh, even if its status marker stays the same. Only one request runs at a time;
results for an obsolete selection or view are discarded. Git uses NUL-delimited,
literal paths. Rendering and navigation do not run Git commands. See
[AGENTS.md](AGENTS.md) for the module boundaries.

Git optional locks, filesystem monitor hooks, external diff programs, and text
conversion are disabled. Untracked and special index-deletion comparisons use
temporary snapshots outside the repository, removed after use. No staging,
editing, or history operations are provided.

Preview limits: patches above 8 MiB, and untracked working files above 8 MiB, get
a summary. Non-UTF-8 filenames remain usable on Unix; their display and non-UTF-8
text content use replacement characters. Control characters are escaped on screen.
Syntax coloring, filesystem watchers, and history browsing are future work.

The 13 behavior tests cover real temporary Git repositories, linked worktrees,
unusual paths, read-only index access, repeated edits, outdated refresh results,
keyboard/mouse navigation, resizing, both layouts, and change colors.

Validation on 2026-11-17: formatting, Clippy with warnings denied, and all tests
passed. Automated terminal smoke checks passed in an isolated plain PTY, a real
WezTerm window, and an isolated Zellij 0.44.3 session using the local configuration.
They exercised keyboard input, mouse-wheel reports, layout switching, live edits,
and exit. A live edit in WezTerm preserved row 7. Terminal settings matched their
pre-launch values after exit; startup outside a Git repository also left them
unchanged. These checks do not replace a human visual check or physical mouse use.
