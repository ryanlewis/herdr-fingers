# Changelog

All notable changes to herdr-fingers are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the versions
[Semantic Versioning](https://semver.org/). Entries describe what a user of
the plugin notices; the build pipeline and the docs live in the commit history.

## [Unreleased]

### Added
- A `links` action hints links printed under a label (`PR #42`, a file
  name), asking Herdr for the address under coloured or underlined text.
  The keys act on the address; a `file://` link opens as the file it names.
  Without Herdr's answer it falls back to the usual hints with a notice.

## [0.3.0] - 2026-10-15

### Added
- `:open:` finds a relative path that is not in the pane's directory: it
  also looks in that directory's Git root, in the folders named on the
  screen (nearest first), and in the new `search_roots` setting (default
  `["~/dev/*"]`), where it opens only when exactly one folder has the path.
- `:open:` on a Git SHA outside a GitHub repository uses the nearest
  GitHub link or GitHub repository named on the screen.
- `:open:` opens a custom pattern's match that names an existing file or
  folder, such as `README.md`, even without a `/`.

### Changed
- Scripts and executable files open in the editor instead of being shown
  in their folder. App bundles and installers are still only shown, and
  with no editor every file is still only shown.

## [0.2.0] - 2026-10-14

### Added
- `herdr plugin install ryanlewis/herdr-fingers` downloads a prebuilt
  binary for macOS (Apple silicon and Intel) and Linux (x86_64 and aarch64)
  and checks its SHA-256, so no Rust toolchain is needed. It builds with
  cargo when there is no binary for the platform or the download fails.
- The overlay's status line lists what each key does, built from the
  configured actions (`hint open · ⇧ paste · ⌃ copy …`). Turn it off with
  `show_legend = false`.
- `:open:` works out what the picked text is: web links open in the browser,
  folders with the system opener, files in your editor in a pane beside the
  source pane (at the line, for `file:12:3`), Git SHAs on their GitHub
  commit page. Programs and app bundles are shown in their folder, never
  run; other link schemes and missing files are refused with a notice.
- `open_url` on a custom pattern sends its matches to a URL, for instance
  ticket keys such as `RET-1234` to your tracker.
- `editor` and `editor_split` settings; the editor defaults to `$VISUAL`,
  then `$EDITOR`.

### Changed
- The plugin id is now `io.rlew.fingers`, and the plugin installs from
  `ryanlewis/herdr-fingers`. Update the keybinding to
  `command = "io.rlew.fingers.start"`, and move an existing `config.toml`
  to the directory `herdr plugin config-dir io.rlew.fingers` prints.
- The log no longer holds the text you pick. It records the action and the
  length; `log_picked_text = true` brings the preview back.
- A plain hint now opens the match; `Ctrl` + hint copies it (`main_action`
  is `:open:` and `ctrl_action` is `:copy:` by default). Set them back in
  `config.toml` to keep the old keys.

## [0.1.0] - 2026-09-22

### Added
- Hints over everything worth copying in the focused Herdr pane: paths,
  URLs, Git SHAs, IPv4 addresses, UUIDs, hex numbers, long numbers,
  Kubernetes resources and pod names, `git status` paths and branch names,
  diff headers — the tmux-fingers built-ins, ported one for one.
- Type a hint to copy its text through Herdr to the terminal you are
  looking at (OSC 52), so it works over SSH; hold Shift to type it into the
  pane instead, Ctrl to open it with the system opener, Alt for a custom
  command. Every action is configurable, including arbitrary shell commands
  fed on stdin with `MODIFIER` and `HINT` in their environment.
- Multi-select with Tab: pick several hints, confirm with Tab or Enter, get
  them joined by a configurable separator.
- The overlay redraws the pane where it is, with its own colors, even when
  the tab is split; a URL or path wrapped over two rows is one hint.
- Identical texts share one hint, and the shortest hints go to the bottom
  of the screen, where the freshest output is.
- Keyboard layouts from tmux-fingers (qwerty, azerty, qwertz, dvorak,
  colemak and their home-row and one-hand variants), or your own alphabet.
- A commented `config.toml` written on first run; custom patterns in Rust
  regex syntax with an optional `match` group.
- `herdr-fingers scan` to preview the hints a screen dump would get.
