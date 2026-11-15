param([string]$Output = "licenses/rust")
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
$state = Join-Path $env:LOCALAPPDATA "LocalRealmBuild"
if (Test-Path -LiteralPath "$state/cargo/bin/cargo.exe") {
    $env:CARGO_HOME = Join-Path $state "cargo"
    $env:RUSTUP_HOME = Join-Path $state "rustup"
    $env:PATH = "$env:CARGO_HOME/bin;$env:PATH"
}
$destination = Join-Path $root $Output
New-Item -ItemType Directory -Path $destination -Force | Out-Null
$metadata = (& cargo metadata --locked --format-version 1 --manifest-path "$root/src-tauri/Cargo.toml" | ConvertFrom-Json)
if ($LASTEXITCODE) { throw "Cargo metadata failed" }
$lines = [System.Collections.Generic.List[string]]::new()
$lines.Add("# Rust dependency licenses")
$lines.Add("")
$lines.Add("Generated from Cargo.lock. License texts are grouped by package and version.")
$lines.Add("")
$lines.Add("| Package | Version | License | Source |")
$lines.Add("| --- | --- | --- | --- |")
foreach ($package in ($metadata.packages | Sort-Object name,version)) {
    if (-not $package.source) { continue }
    $packageRoot = Split-Path $package.manifest_path
    $name = "$($package.name)-$($package.version)"
    $packageOut = Join-Path $destination $name
    $texts = @(Get-ChildItem -LiteralPath $packageRoot -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' })
    if ($package.license_file) {
        $licenseFile = Join-Path $packageRoot $package.license_file
        if (Test-Path -LiteralPath $licenseFile -PathType Leaf) { $texts += Get-Item -LiteralPath $licenseFile }
    }
    if ($texts.Count) {
        New-Item -ItemType Directory -Path $packageOut -Force | Out-Null
        foreach ($file in $texts) { Copy-Item -LiteralPath $file.FullName -Destination $packageOut -Force }
    }
    $license = "$($package.license)".Replace('|',' / ')
    $lines.Add("| $($package.name) | $($package.version) | $license | $($package.repository) |")
}
$lines | Set-Content -LiteralPath (Join-Path $destination "README.md") -Encoding utf8
Write-Output "Recorded $($metadata.packages.Count) packages."
