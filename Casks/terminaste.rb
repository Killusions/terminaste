cask "terminaste" do
  version :latest
  sha256 :no_check

  url "https://github.com/Killusions/terminaste/releases/latest/download/terminaste-macos-arm64.tar.gz"
  name "terminaste"
  desc "Modern terminal with command blocks, completion, tabs, and split panes"
  homepage "https://github.com/Killusions/terminaste"

  depends_on arch: :arm64
  depends_on macos: :big_sur

  app "terminaste.app"

  caveats <<~EOS
    terminaste is currently distributed without Apple notarization.

    Install with --no-quarantine so macOS can launch the unsigned/ad-hoc-signed app:
      brew install --cask --no-quarantine killusions/terminaste/terminaste

    The release has only been tested with zsh on macOS on Apple silicon.
  EOS
end
