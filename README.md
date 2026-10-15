# herdr-fingers

**tmux-fingers for [Herdr](https://herdr.dev).** Press a key, every path,
URL, Git SHA, IP, UUID and number on screen gets a one- or two-letter hint;
type the hint to open it. Hold Ctrl to copy it instead, Shift to type it
into the pane. No mouse, no selection dragging, no scrolling back to find
the thing.

[![CI](https://github.com/ryanlewis/herdr-fingers/actions/workflows/ci.yml/badge.svg)](https://github.com/ryanlewis/herdr-fingers/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-2ea44f)](LICENSE)
![Herdr 0.7+](https://img.shields.io/badge/Herdr-0.7%2B-66b3ff)
![Linux and macOS](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-c084fc)

![herdr-fingers in action: hints over a git status, a path pasted with Shift, two commits multi-selected, URLs labelled](assets/demo.gif)

*Copy a path, paste another with Shift, multi-select two commits with Tab, label URLs. [Watch it as a video.](https://github.com/ryanlewis/herdr-fingers/raw/main/assets/demo.mp4)*

A port of [tmux-fingers](https://github.com/Morantron/tmux-fingers) to
Herdr, written in Rust from the ground up: same patterns, same keyboard
layouts, same actions and multi-select, running as a native Herdr plugin.

## Install

```sh
herdr plugin install ryanlewis/herdr-fingers
```

Herdr clones the repository and runs `install.sh`, which downloads the
prebuilt binary for your platform from the matching GitHub release and
checks its SHA-256. Prebuilt binaries cover macOS (Apple silicon and Intel)
and Linux (x86_64 and aarch64). If there is no binary for your platform or
the download fails, it builds from source with cargo instead (Rust from
[rustup.rs](https://rustup.rs)).

Then bind a key in `~/.config/herdr/config.toml`:

```toml
[[keys.command]]
key = "prefix+f"
type = "plugin_action"
command = "io.rlew.fingers.start"
description = "copy with hints"
```

Reload and try it:

```sh
herdr server reload-config
```

Press `prefix+f` (`ctrl+b` then `f` by default) in any pane.

## Use

Hints appear over every match. The shortest hints sit at the bottom of the
screen, where the freshest output is; identical texts share one hint.

| Key | Does |
|---|---|
| hint (`a`, `sd`…) | open the match (see [Opening](#opening)) |
| `Shift` + hint | type the match into the pane |
| `Ctrl` + hint | copy the match to the clipboard |
| `Alt` + hint | custom action (none by default) |
| `Tab` | toggle multi-select: pick several hints, then `Tab` or `Enter` |
| `Backspace` | erase the last typed key |
| `?` | help |
| `Esc`, `q`, `Ctrl+C` | close |

The status line lists what each key does with your configuration, for
example `hint open · ⇧ paste · ⌃ copy · tab multi · esc quit · ? help`.
`⇧`, `⌃` and `⌥` stand for Shift, Ctrl and Alt; a modifier bound to `""` is
left out, and a command shows its program name. It sits outside the pane or
after the text on the pane's last row, so it never covers a match. When the
line is short of room, the Alt, Ctrl and Shift entries go first, then
`tab multi` (kept while multi-select is on) and the plain hint entry; a
configuration notice, the typed keys, `esc quit` and `? help` stay longest. The help box
(`?`) lists the same actions. Turn the legend off with `show_legend = false`.

Typing the first key of a two-key hint hides every other hint. A URL or path
the terminal wrapped over two rows is one hint, and copies without the line
break. The overlay redraws the pane exactly where it is — with its own
colors — even when the tab is split.

## What gets a hint

The tmux-fingers built-ins, ported one for one:

| Name | Matches |
|---|---|
| `ip` | IPv4 addresses |
| `uuid` | UUIDs |
| `sha` | Git SHAs (7 to 128 hex digits) |
| `digit` | numbers of four digits or more |
| `url` | `http(s)://`, `git@`, `git://`, `ssh://`, `ftp://`, `file:///` |
| `path` | anything with a `/` in it: `src/main.rs`, `~/.config`, `/etc/hosts`, with a `:line` or `:line:col` after it as compilers and grep print (`src/main.rs:12:3`) |
| `hex` | `0x…` numbers |
| `kubernetes` | Kubernetes resource names (`configmap/…`, `deployment.apps/…`) |
| `kubernetes-pod` | deployment-managed pod names (`nginx-66b6c48dd5-7xb2r`) |
| `git-status` | the path on a `modified:` / `new file:` / `deleted:` line |
| `git-status-branch` | the remote branch in `Your branch is up to date with '…'` |
| `diff` | the path in a `--- a/…` / `+++ b/…` header |

When two patterns start at the same column the earlier one in this table
wins; custom patterns come before all of them.

## Configure

The plugin writes a fully commented `config.toml` on first run. Find it with:

```sh
herdr plugin config-dir io.rlew.fingers
```

Every key is optional — [`examples/config.toml`](examples/config.toml) lists
them all with their defaults. The ones people change:

```toml
keyboard_layout = "azerty"          # or qwerty-homerow, dvorak, colemak, …
# alphabet = "asdfghjkl"            # your own keys instead of a layout
hint_position = "left"              # or "right"

main_action = ":open:"              # plain hint
shift_action = ":paste:"            # Shift + hint
ctrl_action = ":copy:"              # Ctrl + hint
alt_action = ""                     # Alt + hint; e.g. "xargs nvim" or a script

editor = "nvim +{line} {path}"      # files opened with :open: (default: $VISUAL, $EDITOR)
editor_split = "right"              # or "down"

enabled_builtin_patterns = ["url", "path", "sha", "git-status"]

[[patterns]]
name = "ticket"
regex = "PROJ-[0-9]+"

[[patterns]]
name = "jira"                       # open ticket keys such as RET-1234
regex = "\\b(?P<match>[A-Z][A-Z0-9]+-[0-9]+)\\b"
open_url = "https://example.atlassian.net/browse/{}"

[[patterns]]
name = "env"
regex = "env=(?P<match>[a-z0-9_-]+)"   # only the group is copied

[style]
hint = { fg = "black", bg = "yellow", bold = true }
highlight = { fg = "yellow" }
backdrop = { dim = true }           # dim everything that is not a match
```

### Actions

`:copy:`, `:paste:` and `:open:` are built in; `""` does nothing. Anything
else is a command line, run from the pane's working directory with the
picked text on its **stdin** and two environment variables, exactly like
tmux-fingers: `MODIFIER` (`main`, `shift`, `ctrl` or `alt`) and `HINT`.

```toml
alt_action = "sh -c 'open \"https://github.com/search?q=$(cat)\"'"
```

In multi-select mode the texts are joined with `multi_separator` (a space
by default) and handed to the action of the last modifier used.

### Opening

`:open:` works out what the picked text is and does the matching thing.
The first rule that applies wins:

1. **A pattern with `open_url`**: the URL, with `{}` replaced by the match
   (URL-encoded). `open_url` must start with `https://` or `http://` and
   contain `{}`.
2. **A web link** (`http://`, `https://`): the browser, through `open` on
   macOS or `xdg-open` on Linux. Other links (`file://`, `ssh://`, `ftp://`,
   `git://`, `git@…`) are not opened.
3. **A Git SHA**: its commit page, when the pane's directory is in a Git
   repository whose `origin` is on github.com. Otherwise the SHA is copied.
4. **A path**: a trailing `:line` or `:line:col` (as in compiler and grep
   output) is kept as the line to jump to; `~` is your home directory; a
   relative path is taken from the pane's working directory, and a diff's
   `a/` or `b/` prefix is dropped when the prefixed path does not exist.
   - a folder opens with the system opener (Finder on macOS);
   - a program — an executable file, or a `.app`, `.command`, `.terminal`,
     `.tool`, `.workflow`, `.pkg`, `.dmg` or `.sh` — is never run: it is
     shown in its folder instead (`open -R` on macOS, the folder on Linux);
   - any other file opens in your editor, in a new pane beside the one
     the hints came from.
5. Anything else: nothing.

When nothing can be opened (a missing file, a refused link, a number), the
hints stay up and the status line says why, so you can press another key —
`Ctrl` + hint to copy, for instance. With several hints picked, nothing is
opened unless all of them can be.

The editor is `editor` from the config, else `$VISUAL`, else `$EDITOR`.
`{path}` is replaced by the file and `{line}` by the line number; a word
that holds `{line}` is left out when there is no line. Without placeholders
the editor gets `+{line} {path}`, which vim, neovim, emacs, nano and helix
understand. The file name is always passed as one argument and never goes
through a shell. The editor pane closes when the editor exits. With no
editor at all, files are shown in their folder.

```toml
editor = "code --wait {path}"   # {path} must be a word of its own
editor_split = "down"
```

### Clipboard

By default `:copy:` sends the text as an **OSC 52** sequence. Herdr forwards
it to the terminal you are attached from, so it lands on *your* clipboard
even when the Herdr server runs on another machine over SSH. Most terminals
accept it out of the box (Ghostty, Kitty, WezTerm, iTerm2, Alacritty, foot,
Windows Terminal); tmux and some others need it enabled. Set
`clipboard = "system"` to use `pbcopy`, `wl-copy`, `xclip` or `xsel` on the
machine running Herdr instead, or `"both"`.

Herdr can show a toast when a pane sets the clipboard
(`[ui.toast.clipboard]` in Herdr's config); `show_copied_notification = true`
adds a toast with the copied text through Herdr's notification system.

### Privacy

The log (`herdr-fingers.log` in the plugin's state directory) records what was
done and how much, such as `copied 1 item (42 chars)`, never the picked text.
Errors from `open` name the opener and not the target. Set
`log_picked_text = true` to log a short preview of the text as well.

### Layouts

`keyboard_layout` takes the tmux-fingers layouts — `qwerty`, `azerty`,
`qwertz`, `dvorak`, `colemak`, each with `-homerow`, `-left-hand` and
`-right-hand` variants. The keys `c`, `i`, `m`, `n` and `q` are never used
for hints, so they cannot collide with the overlay's controls.

## Differences from tmux-fingers

- **No jump mode.** tmux-fingers can move the copy-mode cursor onto a match;
  Herdr's API does not expose that. Everything else maps: main/shift/ctrl/alt
  actions, `:copy:`/`:paste:`/`:open:`, multi-select, custom patterns, layouts.
- **Rust regex syntax** for custom patterns (no look-around, no
  back-references); named groups are `(?P<match>…)` or `(?<match>…)`.
- **Colors are kept.** The overlay redraws the pane with its own styling.

## Troubleshooting

```sh
herdr plugin list                                            # is it there and enabled?
herdr plugin action list --plugin io.rlew.fingers
herdr plugin log list --plugin io.rlew.fingers               # what Herdr saw when it ran the action
tail "$(herdr plugin config-dir io.rlew.fingers | sed 's#/config/#/state/#')/herdr-fingers.log"
```

Nothing happens on the key? Check the action id in your `[[keys.command]]`
block and run `herdr server reload-config`. The overlay opens but shows
"Nothing to pick"? Try the pattern engine on a dump of the pane:

```sh
herdr pane read <pane-id> --source visible --format ansi > dump.txt
herdr-fingers scan --width <pane width> < dump.txt      # hint, row:col, text
```

`herdr-fingers` is `target/release/herdr-fingers` inside the directory
`herdr plugin list` prints as the plugin root.

## Requirements

- Herdr 0.7 or newer, on Linux or macOS
- A Rust toolchain (1.85+) only when no prebuilt binary fits, or to build
  from source

## Build from source

```sh
git clone https://github.com/ryanlewis/herdr-fingers.git
cd herdr-fingers
cargo build --release
herdr plugin link "$PWD"
```

`cargo test` runs the suite: the ANSI parser, wrapped-row joining, every
built-in pattern, hint generation, the picking state machine, the two use
cases against in-memory fakes, the config loader, the Herdr client against
a fake socket, and the renderer on a ratatui test backend. `cargo fmt --check` and
`cargo clippy --all-targets -- -D warnings` must be clean; CI runs all of it
on Linux and macOS.

The code follows Clean Architecture in one crate: a pure kernel under
`src/domain/`, use cases under `src/usecases/` that only see ports, and
adapters implementing those ports for Herdr, the terminal, the clipboard
and processes. A test enforces the Dependency Rule. See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Contributing

Start with [CONTRIBUTING.md](CONTRIBUTING.md). Bugs and ideas go through
the issue templates; security flaws through [SECURITY.md](SECURITY.md).
Everyone taking part is held to the [code of conduct](CODE_OF_CONDUCT.md).

## Credits

Based on [herdr-fingers](https://github.com/nathan-poncet/herdr-fingers) by
Nathan Poncet, itself a port of
[tmux-fingers](https://github.com/Morantron/tmux-fingers). MIT licensed; see
[LICENSE](LICENSE).

## License

[MIT](LICENSE).
