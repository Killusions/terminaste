cask "terminaste" do
  version "0.2.0"
  sha256 :no_check

  url "https://github.com/Killusions/terminaste/releases/download/v#{version}/terminaste-macos-arm64.tar.gz"
  name "terminaste"
  desc "Modern terminal with a visual IDE-style input, built native"
  homepage "https://github.com/Killusions/terminaste"

  depends_on arch: :arm64
  depends_on macos: :big_sur

  app "terminaste.app"

  caveats <<~EOS
    terminaste is currently distributed without code signing or Apple notarization.

    Remove quarantine after installing so macOS can launch it:
      xattr -dr com.apple.quarantine /Applications/terminaste.app

    zsh on macOS with Apple silicon is tested most. Other supported shells and
    platforms are largely untested.
  EOS
end
