# terminaste

> A modern terminal you'll love.

terminaste brings a modern UX to the terminal. It has a visual, IDE-style input
and turns commands and output into clear, parseable blocks. It is built in Rust
with GPUI for native GPU rendering and a fast, responsive feel.

It stays minimal: just a terminal, with an open MIT license.

> [!IMPORTANT]
> terminaste is early software. zsh on macOS with Apple silicon is tested most.
> Bash, fish, PowerShell, Linux, Windows, and Intel Mac are supported but still
> largely untested.

## Modern terminal UX

- A fancy IDE-style input stays at the bottom of the window.
- Commands, output, exit status, and duration form parseable blocks.
- Tabs and horizontal or vertical split panes help with parallel work.
- Search, filters, and selection make old output easy to use.
- Copy a whole block, only its command, or only its output.
- Shell history, command completion, and path completion stay close to the input.
- A command palette and keyboard shortcuts keep the modern UX quick and simple.
- Sessions restore tabs and panes after a restart.

## Native and responsive

terminaste is a native Rust app built with GPUI and native GPU rendering. It does
not use Electron or a browser engine. Input, PTY work, parsing, and rendering live
in small focused crates. Release builds use thin LTO, and scrollback has a limit
to keep long sessions under control.

There are no benchmark claims yet. The goal is a responsive terminal with modern
UX and minimal overhead.

## Install with Homebrew

The Homebrew release is only for Apple silicon Macs. It is not Apple-notarized,
so use `--no-quarantine` to open the unsigned/ad-hoc-signed app:

```sh
brew tap killusions/terminaste https://github.com/Killusions/terminaste
brew install --cask --no-quarantine killusions/terminaste/terminaste
```

Update or remove it with:

```sh
brew upgrade --cask --no-quarantine terminaste
brew uninstall --cask terminaste
```

## Download

Download `terminaste-macos-arm64.tar.gz` from the
[latest release](https://github.com/Killusions/terminaste/releases/latest), unpack
it, and move `terminaste.app` to `/Applications`.

If macOS still blocks the app, remove quarantine from this app only:

```sh
xattr -dr com.apple.quarantine /Applications/terminaste.app
```

Only do this for a file from the official release page.

## Build and run

Install stable Rust and the macOS command-line developer tools, then run:

```sh
cargo run --release -p terminaste-app
```

Build the macOS app, archive, and checksum with:

```sh
tools/release/macos
```

Run all formatting, lint, asset, and test checks with:

```sh
tools/check.sh
```

## Rust crates

These small building blocks can be published to crates.io:

- `terminaste-core`
- `terminaste-completion`
- `terminaste-settings`
- `terminaste-pty`

Check the packages locally:

```sh
tools/release/publish-crates
```

Publish the workspace version in the correct order:

```sh
tools/release/publish-crates --execute
```

The app and UI crates stay in this repository because GPUI is pinned to a Git
revision.

## License

terminaste uses the [MIT License](LICENSE). Third-party notices are in
[THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md).
