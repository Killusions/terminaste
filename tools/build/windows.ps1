$ErrorActionPreference = "Stop"

$RootDir = Resolve-Path (Join-Path $PSScriptRoot "../..")
$DistDir = Join-Path $RootDir "dist/terminaste-windows"
$BinPath = Join-Path $RootDir "target/ship/terminaste.exe"

cargo run -p terminaste-tools -- assets
cargo build --profile ship --bin terminaste

if (Test-Path $DistDir) {
    Remove-Item -Recurse -Force $DistDir
}
New-Item -ItemType Directory -Force -Path (Join-Path $DistDir "bin") | Out-Null
New-Item -ItemType Directory -Force -Path (Join-Path $DistDir "assets") | Out-Null
Copy-Item $BinPath (Join-Path $DistDir "bin/terminaste.exe")
Copy-Item (Join-Path $RootDir "LICENSE") (Join-Path $DistDir "LICENSE")
Copy-Item (Join-Path $RootDir "README.md") (Join-Path $DistDir "README.md")
Copy-Item (Join-Path $RootDir "assets/settings.schema.json") (Join-Path $DistDir "assets/settings.schema.json")
Copy-Item (Join-Path $RootDir "assets/icon.png") (Join-Path $DistDir "assets/icon.png")
Copy-Item -Recurse (Join-Path $RootDir "assets/icons") (Join-Path $DistDir "assets/icons")
Copy-Item (Join-Path $RootDir "assets/THIRD-PARTY-LICENSES.md") (Join-Path $DistDir "THIRD-PARTY-LICENSES.md")
Copy-Item (Join-Path $RootDir "assets/dependency-manifest.json") (Join-Path $DistDir "dependency-manifest.json")

Write-Host "Packaged $DistDir"
