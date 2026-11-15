param([ValidateSet("build","test","check","release")][string]$Action = "release", [string]$Reference)
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot
Push-Location -LiteralPath $root
try {
    node tools/build-frontend.mjs
    if ($LASTEXITCODE) { throw "Frontend build failed" }
    $state = Join-Path $env:LOCALAPPDATA "LocalRealmBuild"
    $manifest = Join-Path $root "src-tauri/Cargo.toml"
    if (Test-Path -LiteralPath "$state/cargo/bin/cargo.exe") {
        $env:CARGO_HOME = Join-Path $state "cargo"
        $env:RUSTUP_HOME = Join-Path $state "rustup"
        $env:PATH = "$state\w64devkit\bin;$env:CARGO_HOME\bin;$env:PATH"
        $env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "x86_64-w64-mingw32-gcc"
        $sysroot = rustc --print sysroot
        $env:LIBRARY_PATH = Join-Path $sysroot "lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained"
        $env:CARGO_TARGET_DIR = Join-Path $state "target"
        # GNU windres does not handle this project's Korean directory name.
        $mirror = Join-Path $state "source"
        foreach ($folder in "src-tauri","dist","recovered-backend") {
            New-Item -ItemType Directory -Path (Join-Path $mirror $folder) -Force | Out-Null
        }
        Copy-Item src-tauri/Cargo.*,src-tauri/build.rs,src-tauri/tauri.conf.json "$mirror/src-tauri" -Force
        foreach ($folder in "src","icons","capabilities","resources") {
            Copy-Item "src-tauri/$folder" "$mirror/src-tauri" -Recurse -Force
        }
        Copy-Item recovered-backend/migrations "$mirror/recovered-backend" -Recurse -Force
        Copy-Item dist/frontend "$mirror/dist" -Recurse -Force
        $manifest = Join-Path $mirror "src-tauri/Cargo.toml"
    }
    [string[]]$cargoArgs = switch ($Action) { "test" { @("test") }; "check" { @("clippy","--all-targets") }; "build" { @("build") }; "release" { @("build","--release") } }
    if ($Action -eq "check") {
        & cargo @cargoArgs --locked --manifest-path $manifest -- -D warnings
    } else {
        & cargo @cargoArgs --locked --manifest-path $manifest
    }
    if ($LASTEXITCODE) { throw "Cargo $Action failed" }
    if ($Action -eq "test" -and $Reference) {
        $env:LOCAL_REALM_REFERENCE = (Resolve-Path -LiteralPath $Reference).Path
        cargo test --locked --manifest-path $manifest -- --ignored
        if ($LASTEXITCODE) { throw "Reference compatibility test failed" }
    }
    if ($Action -in "build","release") {
        $target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $root "src-tauri/target" }
        $profile = if ($Action -eq "release") { "release" } else { "debug" }
        $out = Join-Path $root "dist/rebuilt"
        New-Item -ItemType Directory -Force $out | Out-Null
        Copy-Item "$target/$profile/local-realm-rebuilt.exe" $out -Force
        Copy-Item assets/brushes $out -Recurse -Force
        $loader = Join-Path $target "$profile/WebView2Loader.dll"
        if (Test-Path -LiteralPath $loader) { Copy-Item -LiteralPath $loader -Destination $out -Force }
        Write-Output (Join-Path $out "local-realm-rebuilt.exe")
    }
} finally { Pop-Location }

