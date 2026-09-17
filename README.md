# terminaste

> A modern terminal you'll love.

terminaste is a native Rust terminal that treats commands as useful, searchable
blocks instead of an undifferentiated stream of text. It is intentionally small:
one app, one settings file, and no account, background service, or web runtime.

> [!IMPORTANT]
> terminaste is early software. It has only been tested with zsh on macOS on
> Apple silicon. Bash, fish, PowerShell, Linux, Windows, and Intel Mac support are
> present in parts of the codebase but are not release-tested yet.

## Functionality

- Command blocks keep the command, output, exit status, and duration together.
- Tabs and resizable horizontal or vertical split panes support parallel work.
- Search, block filtering, selection, and dedicated copy-command/copy-output
  actions make old terminal output useful again.
- Shell-aware history suggestions and command/path completion stay beside the
  input instead of taking over the screen.
- A command palette, editable keybindings, appearance controls, and live-reloaded
  TOML settings keep common actions close without filling the UI with chrome.
- Session state restores tabs and panes between launches.

## UX

The input remains anchored at the bottom while completed commands form a readable
history above it. Output can still be selected like a normal terminal, while a
whole command block, its command, or only its output can also be copied directly.
Keyboard-first actions cover navigation, panes, tabs, search, and settings.

The bundled JetBrains Mono font gives a consistent first launch. Dark, light, and
system appearance modes are available, and settings live in
`~/Library/Application Support/com.terminaste.terminaste/settings.toml` on macOS.

## Performance and minimalness

terminaste is a native Rust application rendered with GPUI. It does not embed
Electron or a browser engine. PTY I/O, terminal parsing, completion, and rendering
are separated into focused crates; release builds use thin LTO and a single code
generation unit. Scrollback has a configurable bound so long sessions do not grow
without limit.

There are no benchmark promises yet. The current goal is immediate input,
streaming output, and stable suggestions with as little machinery as practical.

## Install

### Homebrew

The current macOS release is not Apple-notarized. Install the cask without
quarantine so macOS can launch the unsigned/ad-hoc-signed executable:

```sh
brew tap killusions/terminaste https://github.com/Killusions/terminaste
brew install --cask --no-quarantine killusions/terminaste/terminaste
```

Upgrade or remove it with:

```sh
brew upgrade --cask --no-quarantine terminaste
brew uninstall --cask terminaste
```

### Download

Download `terminaste-macos-arm64.tar.gz` from the
[latest release](https://github.com/Killusions/terminaste/releases/latest), extract
it, and move `terminaste.app` to `/Applications`.

If the archive was downloaded by a browser and macOS says the app cannot be
verified, remove the quarantine attribute from this app only, then open it again:

```sh
xattr -dr com.apple.quarantine /Applications/terminaste.app
```

Only do this for an archive downloaded from the official release page.

### Build and run

Install the stable Rust toolchain and the macOS command-line developer tools,
then run:

```sh
cargo run --release -p terminaste-app
```

Create the distributable app and archive with:

```sh
tools/release/macos
```

The archive and SHA-256 checksum are written to `dist/`.

## Reusable crates

The UI and application crates depend on a pinned GPUI Git revision and are not
published to crates.io. The independent building blocks are publishable:

- `terminaste-core`
- `terminaste-completion`
- `terminaste-settings`
- `terminaste-pty`

Validate their packages locally:

```sh
tools/release/publish-crates
```

Maintainers can publish the current workspace version in dependency order with
`tools/release/publish-crates --execute` after setting `CARGO_REGISTRY_TOKEN` or
logging in with `cargo login`.

## Development

Run formatting, lints, generated-asset checks, and unit/integration tests with:

```sh
tools/check.sh
```

## License

terminaste is released under the [MIT License](LICENSE). Bundled fonts and other
dependency notices are listed in [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
