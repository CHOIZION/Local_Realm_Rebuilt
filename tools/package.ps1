param([string]$RuntimePath, [switch]$SkipBuild)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
Push-Location -LiteralPath $root
try {
    if (-not $SkipBuild) { & "$PSScriptRoot/build.ps1" -Action release }
    foreach ($required in "dist/rebuilt/local-realm-rebuilt.exe","dist/rebuilt/WebView2Loader.dll","dist/rebuilt/brushes","licenses/rust/README.md","licenses/fonts/NanumGothic-OFL.txt") {
        if (-not (Test-Path -LiteralPath (Join-Path $root $required))) { throw "Missing release input: $required" }
    }
    $version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
    $name = "Local-Realm-Rebuilt-$version"
    $out = Join-Path $root "deliverables"
    New-Item -ItemType Directory -Path $out -Force | Out-Null
    $stage = Join-Path ([System.IO.Path]::GetTempPath()) ("localrealm-package-" + [guid]::NewGuid().ToString("N"))
    $source = Join-Path $stage "$name-source"
    $portable = Join-Path $stage "$name-windows-x64"
    New-Item -ItemType Directory -Path $source,$portable -Force | Out-Null
    # An allowlist keeps old binary-patching experiments, local fixtures and caches out of GitHub.
    foreach ($folder in ".github","frontend","recovered-frontend","recovered-backend","assets","licenses","docs") {
        Copy-Item -LiteralPath (Join-Path $root $folder) -Destination $source -Recurse -Force
    }
    foreach ($folder in "src-tauri","tests","tools") { New-Item -ItemType Directory -Path (Join-Path $source $folder) -Force | Out-Null }
    foreach ($folder in "src","icons","capabilities","resources") { Copy-Item "src-tauri/$folder" "$source/src-tauri" -Recurse -Force }
    Copy-Item src-tauri/Cargo.toml,src-tauri/Cargo.lock,src-tauri/build.rs,src-tauri/tauri.conf.json "$source/src-tauri" -Force
    Copy-Item tests/*.mjs "$source/tests" -Force
    foreach ($tool in "build-frontend.mjs","build.ps1","package.ps1","collect-licenses.ps1") { Copy-Item "tools/$tool" "$source/tools" -Force }
    Copy-Item .gitignore,README.md,THIRD_PARTY_NOTICES.md,package.json "$source" -Force
    Copy-Item dist/rebuilt/* $portable -Recurse -Force
    Copy-Item THIRD_PARTY_NOTICES.md $portable -Force
    Copy-Item licenses $portable -Recurse -Force
    Copy-Item docs/RELEASE.md "$portable/README.md" -Force
    Copy-Item docs/COMPATIBILITY.md $portable -Force
    Copy-Item docs/VALIDATION.md $portable -Force
    if ($RuntimePath) {
        $runtime = Get-Item -LiteralPath $RuntimePath
        if (-not $runtime.PSIsContainer -or -not (Test-Path -LiteralPath (Join-Path $runtime.FullName "msedgewebview2.exe"))) { throw "Not a WebView2 fixed runtime folder" }
        Copy-Item -LiteralPath $runtime.FullName -Destination $portable -Recurse -Force
    }
    $sourceZip = Join-Path $out "$name-source.zip"
    $releaseZip = Join-Path $out "$name-windows-x64.zip"
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    foreach ($pair in @(@($source,$sourceZip),@($portable,$releaseZip))) {
        $temporaryArchive = "$($pair[1]).tmp"
        if (Test-Path -LiteralPath $temporaryArchive) { throw "A previous packaging run left $temporaryArchive; inspect it before continuing" }
        [System.IO.Compression.ZipFile]::CreateFromDirectory($pair[0], $temporaryArchive, [System.IO.Compression.CompressionLevel]::Optimal, $true)
        Move-Item -LiteralPath $temporaryArchive -Destination $pair[1] -Force
    }
    Get-FileHash -LiteralPath $sourceZip,$releaseZip -Algorithm SHA256 |
        ForEach-Object { "$($_.Hash.ToLower())  $(Split-Path $_.Path -Leaf)" } |
        Set-Content -LiteralPath (Join-Path $out "SHA256SUMS.txt") -Encoding ascii
    # The staging path is fresh and retained for inspection; packaging never removes user directories.
    Write-Output "Source: $sourceZip"
    Write-Output "Release: $releaseZip"
    Write-Output "Staging: $stage"
} finally { Pop-Location }
