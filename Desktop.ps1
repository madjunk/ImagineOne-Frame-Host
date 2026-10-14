#Requires -Version 5.1
<#
    ImagineOne desktop app helper
    =============================

    The desktop app ("Imagine One IT", and each customer's branded copy) is built with Tauri
    (Rust + the WebView2 that is part of Windows): tauri\ in this folder, with the title bar and
    its screens from src\renderer (npm run build:ui). The Electron version (src\main, src\preload)
    is no longer built.

    Run it with no arguments for a menu (or double-click Desktop.cmd):
        powershell -ExecutionPolicy Bypass -File .\Desktop.ps1

    Or call one action directly:
        .\Desktop.ps1 Setup                      one time: install Rust + the C++ build tools
        .\Desktop.ps1 Key                        one time: make the update signing key (shared with I1 Phone)
        .\Desktop.ps1 Dev                        run it (debug build)
        .\Desktop.ps1 Dev -Server https://nefco.imagineone.it   ...against that server
        .\Desktop.ps1 Build [-Publish]           standard installer -> dist\ImagineOne-Setup.exe
        .\Desktop.ps1 Customer -Customer nefco [-Publish]
                                                 branded installer -> ..\customers\nefco\dist\nefco-Setup.exe
        .\Desktop.ps1 Bump                       raise the version (needed so installed apps update)
        .\Desktop.ps1 Publish [-Customer nefco]  upload the last build to a draft GitHub release
        .\Desktop.ps1 Typecheck                  type check + compile check + unit tests

    The standard installer applies resources\frame-config.json on first run (ImagineOne Studio:
    Desktop app > Installer config). A customer build reads ..\customers\<slug>\profile.json
    ({ name, slug, baseUrl, publishOwner, publishRepo }) and uses their icon.ico / icon.png
    (made from logo.png by ..\tools\make-icons.js).

    Updates come from GitHub releases (publishOwner/publishRepo, default madjunk/ImagineOne-Frame-Host):
    installed apps read update.json (standard) or update-<slug>.json from the latest release and only
    accept installers signed with the update key (%USERPROFILE%\.tauri\i1-phone.key, the same key as
    I1 Phone). Publish also uploads latest.yml / <slug>.yml, which apps still on the Electron version
    read: they update themselves to this version, and its installer removes the Electron app.
    Publish needs a GitHub token in $env:GH_TOKEN (contents: write on that repo); it is never saved.
#>

[CmdletBinding()]
param(
    [Parameter(Position = 0)]
    [ValidateSet('Menu', 'Setup', 'Key', 'Dev', 'Build', 'Customer', 'Bump', 'Publish', 'Typecheck')]
    [string]$Action = 'Menu',
    [string]$Customer = '',
    [string]$Server = '',
    [switch]$Publish
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 1.0

$Here         = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
$Codex        = Split-Path -Parent $Here
$Tools        = Join-Path $Codex 'tools'
$Customers    = Join-Path $Codex 'customers'
$TauriDir     = Join-Path $Here 'tauri'
$SrcTauri     = Join-Path $TauriDir 'src-tauri'
$PackagedCfg  = Join-Path $SrcTauri 'resources\frame-config.json'
$StandardCfg  = Join-Path $Here 'resources\frame-config.json'
$OverlayFile  = Join-Path $SrcTauri 'tauri.build.json'
$CustIcons    = Join-Path $SrcTauri 'icons\customer'
$BundleDir    = Join-Path $SrcTauri 'target\release\bundle\nsis'
$KeyFile      = Join-Path $env:USERPROFILE '.tauri\i1-phone.key'   # one update key for the I1 desktop apps
$TauriCli     = '@tauri-apps/cli@2'
$DefaultOwner = 'madjunk'
$DefaultRepo  = 'ImagineOne-Frame-Host'
$Utf8         = New-Object System.Text.UTF8Encoding($false)

$env:NO_COLOR = '1'
$env:FORCE_COLOR = '0'
# The old Electron packages are still listed for reference; never download Electron itself.
$env:ELECTRON_SKIP_BINARY_DOWNLOAD = '1'

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

function Write-Step([string]$Text) { Write-Host ''; Write-Host "== $Text ==" -ForegroundColor Cyan }
function Write-Ok([string]$Text)   { Write-Host $Text -ForegroundColor Green }
function Write-Warn2([string]$Text) { Write-Host $Text -ForegroundColor Yellow }
function Write-Utf8([string]$Path, [string]$Text) { [System.IO.File]::WriteAllText($Path, $Text, $Utf8) }

function Invoke-Native {
    <# Run a command in a folder (default: the project) and stop if it fails. #>
    param([string]$Label, [scriptblock]$Command, [string]$In = $Here)
    Write-Step $Label
    Push-Location $In
    try {
        & $Command
        if ($LASTEXITCODE -ne 0) { throw "$Label failed (exit code $LASTEXITCODE)." }
    } finally { Pop-Location }
}

function Assert-Node {
    if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
        throw 'Node.js was not found. Install Node 22 LTS from https://nodejs.org and open a new PowerShell window.'
    }
}

function Find-Cargo {
    <# cargo.exe, also right after Setup when PATH in this window is not refreshed yet. #>
    $c = Get-Command cargo -ErrorAction SilentlyContinue
    if ($c) { return $c.Source }
    $p = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
    if (Test-Path $p) {
        $env:PATH = (Split-Path $p) + ';' + $env:PATH
        return $p
    }
    return $null
}

function Test-CppTools {
    <# The Visual Studio C++ build tools Rust needs to link Windows programs. #>
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (-not (Test-Path $vswhere)) { return $false }
    $found = & $vswhere -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2>$null
    return [bool]$found
}

function Assert-BuildTools {
    Assert-Node
    if (-not (Find-Cargo) -or -not (Test-CppTools)) {
        throw 'The build tools are not installed yet (Rust and the C++ build tools). Run Setup first (menu option S, or .\Desktop.ps1 Setup).'
    }
}

function Install-UiDeps {
    <# The title bar is React + Vite (src\renderer): its packages, first run only. #>
    if (-not (Test-Path (Join-Path $Here 'node_modules\vite'))) {
        Invoke-Native 'Installing packages (first run only)' { npm install --no-audit --no-fund }
    }
}

function Get-Version {
    $pkg = Get-Content (Join-Path $Here 'package.json') -Raw | ConvertFrom-Json
    return [string]$pkg.version
}

function Get-PublicKey {
    <# The update key's public half (goes into the app), or '' when there is no key yet. #>
    $pub = "$KeyFile.pub"
    if (Test-Path $pub) { return (Get-Content $pub -Raw).Trim() }
    return ''
}

function Clear-Dist([string]$DistDir) {
    <# Start each build with an empty output folder (old Electron output included). #>
    Write-Step 'Clearing the output folder'
    if (Test-Path $DistDir) {
        for ($i = 1; $i -le 5 -and (Test-Path $DistDir); $i++) {
            try { Remove-Item $DistDir -Recurse -Force -ErrorAction Stop } catch { Start-Sleep -Seconds 2 }
        }
        if (Test-Path $DistDir) { throw "Couldn't clear $DistDir - close the app (and anything else using a file in that folder) and try again." }
    }
    New-Item -ItemType Directory -Path $DistDir | Out-Null
    Write-Host '  ready.'
}

function Get-FeedNames([string]$Slug) {
    <# File names in the release: the installer, the Tauri update feed, the Electron update feed. #>
    if ($Slug) {
        return [pscustomobject]@{ Exe = "$Slug-Setup.exe"; Json = "update-$Slug.json"; Yml = "$Slug.yml" }
    }
    return [pscustomobject]@{ Exe = 'ImagineOne-Setup.exe'; Json = 'update.json'; Yml = 'latest.yml' }
}

function Write-ElectronFeed([string]$Exe, [string]$Version, [string]$YmlName) {
    <#
        latest.yml / <slug>.yml next to the installer: what apps still on the Electron version check
        (electron-updater, GitHub provider). Their next update is this version.
    #>
    $item = Get-Item $Exe
    $hex = (Get-FileHash -Algorithm SHA512 -Path $Exe).Hash
    $bytes = New-Object byte[] ($hex.Length / 2)
    for ($i = 0; $i -lt $bytes.Length; $i++) { $bytes[$i] = [Convert]::ToByte($hex.Substring($i * 2, 2), 16) }
    $sha = [Convert]::ToBase64String($bytes)
    $date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ss.fffZ')
    $yml = @(
        "version: $Version",
        'files:',
        "  - url: $($item.Name)",
        "    sha512: $sha",
        "    size: $($item.Length)",
        "path: $($item.Name)",
        "sha512: $sha",
        "releaseDate: '$date'"
    ) -join "`n"
    Write-Utf8 (Join-Path $item.DirectoryName $YmlName) ($yml + "`n")
}

function Write-TauriFeed([string]$Exe, [string]$Version, [string]$JsonName, [string]$Owner, [string]$Repo, [string]$Title) {
    <# update.json / update-<slug>.json: what the Tauri app checks, pointing at this release's installer. #>
    $item = Get-Item $Exe
    $feed = [ordered]@{
        version = $Version
        notes = "$Title $Version"
        pub_date = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
        platforms = [ordered]@{
            'windows-x86_64' = [ordered]@{
                signature = (Get-Content "$Exe.sig" -Raw).Trim()
                url = "https://github.com/$Owner/$Repo/releases/download/v$Version/$([Uri]::EscapeDataString($item.Name))"
            }
        }
    }
    Write-Utf8 (Join-Path $item.DirectoryName $JsonName) ($feed | ConvertTo-Json -Depth 5)
}

function Invoke-TauriBuild {
    <#
        One installer: standard (no -Name) or a customer's (name, slug, server, icon).
        Copies the result to $DistDir under the same names the Electron builds used
        (ImagineOne-Setup.exe / <slug>-Setup.exe), plus .sig and both update feeds.
    #>
    param([string]$DistDir, [string]$ConfigJson, [string]$Name = '', [string]$Slug = '',
          [string]$Owner = $DefaultOwner, [string]$Repo = $DefaultRepo, [string]$Ico = '', [string]$Png = '')
    Assert-BuildTools
    $version = Get-Version
    $pubKey = Get-PublicKey
    $names = Get-FeedNames $Slug
    if (-not $pubKey) {
        Write-Warn2 '  No update key yet (menu option K): this installer works, but apps installed from it will not update themselves.'
    }

    Clear-Dist $DistDir
    $buildSettings = [ordered]@{}
    $bundle = [ordered]@{}
    if ($Name) {
        # The same product name as the Electron build, so its installer finds and replaces it.
        $buildSettings.productName = $Name
        $buildSettings.identifier = "it.imagineone.frame.$Slug"
    }
    if ($Ico) {
        New-Item -ItemType Directory -Force -Path $CustIcons | Out-Null
        Copy-Item $Ico (Join-Path $CustIcons 'icon.ico') -Force
        Copy-Item $Png (Join-Path $CustIcons 'icon.png') -Force
        $bundle.icon = @('icons/customer/icon.png', 'icons/customer/icon.ico')
        $bundle.windows = @{ nsis = @{ installerIcon = 'icons/customer/icon.ico' } }
    }
    if ($pubKey) {
        $bundle.createUpdaterArtifacts = $true
        $buildSettings.plugins = @{ updater = @{
            pubkey = $pubKey
            endpoints = @("https://github.com/$Owner/$Repo/releases/latest/download/$($names.Json)")
        } }
        $env:TAURI_SIGNING_PRIVATE_KEY = $KeyFile
    } else {
        Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    }
    if ($bundle.Count) { $buildSettings.bundle = $bundle }

    try {
        # The server (and company) the app applies on first run (src-tauri/src/lib.rs, Config).
        Write-Utf8 $PackagedCfg $ConfigJson
        Write-Utf8 $OverlayFile ($buildSettings | ConvertTo-Json -Depth 6)
        Install-UiDeps
        Invoke-Native 'Building the title bar and its screens' { npm run build:ui }
        Invoke-Native 'Unit tests' { cargo test --lib --quiet } -In $SrcTauri
        Invoke-Native "Building the installer (version $version; the first build takes a few minutes)" {
            npx --yes $TauriCli build --ci --config $OverlayFile
        } -In $TauriDir
    } finally {
        Write-Utf8 $PackagedCfg "{}`n"
        Remove-Item $OverlayFile -Force -ErrorAction SilentlyContinue
        Remove-Item $CustIcons -Recurse -Force -ErrorAction SilentlyContinue
        Remove-Item Env:\TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    }

    $setup = Get-ChildItem -Path $BundleDir -Filter "*_${version}_x64-setup.exe" -File |
        Sort-Object LastWriteTime -Descending | Select-Object -First 1
    if (-not $setup) { throw "The build finished but no installer for version $version is in $BundleDir." }
    $target = Join-Path $DistDir $names.Exe
    Copy-Item $setup.FullName $target -Force
    if (Test-Path "$($setup.FullName).sig") {
        Copy-Item "$($setup.FullName).sig" "$target.sig" -Force
        $title = if ($Name) { $Name } else { 'Imagine One IT' }
        Write-TauriFeed $target $version $names.Json $Owner $Repo $title
    }
    Write-ElectronFeed $target $version $names.Yml

    Write-Host ''
    Write-Ok "Installer: $target"
    Write-Host "           ($([Math]::Round((Get-Item $target).Length / 1MB, 1)) MB, version $version)"
    if (Test-Path "$target.sig") { Write-Host "           signed for auto-update from github.com/$Owner/$Repo" }
}

function Get-CustomerProfile([string]$Slug) {
    if (-not $Slug) { $Slug = (Read-Host 'Customer slug (folder name under customers\)').Trim().ToLower() }
    if ($Slug -cnotmatch '^[a-z0-9][a-z0-9-]{0,48}[a-z0-9]$') { throw "'$Slug' is not a valid slug (lowercase letters, digits, dashes)." }
    $folder = Join-Path $Customers $Slug
    $file = Join-Path $folder 'profile.json'
    if (-not (Test-Path $file)) { throw "There is no customer profile at $file. Add the customer in ImagineOne Studio (Customers > New customer) first." }
    $p = Get-Content $file -Raw | ConvertFrom-Json
    $name = [string]$p.name
    $base = [string]$p.baseUrl
    $owner = if ($p.PSObject.Properties['publishOwner'] -and $p.publishOwner) { [string]$p.publishOwner } else { $DefaultOwner }
    $repo = if ($p.PSObject.Properties['publishRepo'] -and $p.publishRepo) { [string]$p.publishRepo } else { $DefaultRepo }
    if (-not $name) { throw "$file has no ""name""." }
    if ($base -notmatch '^https?://[^\s/]+') { throw "baseUrl in $file must be a full http:// or https:// address." }
    if ($owner -notmatch '^[A-Za-z0-9._-]+$' -or $repo -notmatch '^[A-Za-z0-9._-]+$') { throw "publishOwner / publishRepo in $file may only use letters, digits, dots, dashes and underscores." }
    return [pscustomobject]@{ Slug = $Slug; Folder = $folder; Name = $name; BaseUrl = $base.TrimEnd('/'); Owner = $owner; Repo = $repo }
}

function Get-GitHubRelease([string]$Owner, [string]$Repo, [string]$Version, [hashtable]$Headers) {
    <# The release for v<version> (drafts included), made as a draft when it isn't there yet. #>
    $api = "https://api.github.com/repos/$Owner/$Repo"
    $tag = "v$Version"
    $list = Invoke-RestMethod -Uri "$api/releases?per_page=50" -Headers $Headers
    foreach ($r in @($list)) { if ($r.tag_name -eq $tag) { return $r } }
    Write-Host "  making a draft release $tag"
    $body = @{ tag_name = $tag; name = $tag; draft = $true; body = "Desktop app $Version" } | ConvertTo-Json
    return Invoke-RestMethod -Method Post -Uri "$api/releases" -Headers $Headers -Body $body -ContentType 'application/json'
}

function Invoke-PublishBuild {
    <# Upload the last build (standard, or -Customer) to its GitHub release (a draft until you publish it). #>
    $token = [string]$env:GH_TOKEN
    if (-not $token) {
        throw 'No GitHub token: set $env:GH_TOKEN (contents: write on the release repo) in this window first. It is never saved.'
    }
    if ($Customer) {
        $c = Get-CustomerProfile $Customer
        $dist = Join-Path $c.Folder 'dist'
        $owner = $c.Owner; $repo = $c.Repo; $slug = $c.Slug
    } else {
        $dist = Join-Path $Here 'dist'
        $owner = $DefaultOwner; $repo = $DefaultRepo; $slug = ''
    }
    $names = Get-FeedNames $slug
    $version = Get-Version
    $files = @()
    foreach ($f in @($names.Exe, $names.Json, $names.Yml)) {
        $path = Join-Path $dist $f
        if (Test-Path $path) { $files += $path }
    }
    if (-not (Test-Path (Join-Path $dist $names.Exe))) { throw "No build in $dist - build first." }
    if (-not (Test-Path (Join-Path $dist $names.Json))) {
        Write-Warn2 '  This build is not signed (no update key when it was built): new installs work, but installed apps will not update to it.'
    }
    $yml = Join-Path $dist $names.Yml
    if (Test-Path $yml) {
        $builtVersion = ''
        foreach ($line in (Get-Content $yml)) { if ($line -match '^version:\s*(.+)$') { $builtVersion = $Matches[1].Trim() } }
        if ($builtVersion -and $builtVersion -ne $version) { throw "The build in $dist is version $builtVersion, but package.json says $version. Build again first." }
    }

    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    $headers = @{ Authorization = "Bearer $token"; Accept = 'application/vnd.github+json'; 'User-Agent' = 'ImagineOne-Desktop-ps1'; 'X-GitHub-Api-Version' = '2022-11-28' }
    Write-Step "Uploading version $version to github.com/$owner/$repo"
    $release = Get-GitHubRelease $owner $repo $version $headers
    foreach ($path in $files) {
        $leaf = Split-Path $path -Leaf
        foreach ($a in @($release.assets)) {
            if ($a -and $a.name -eq $leaf) {
                Invoke-RestMethod -Method Delete -Uri "https://api.github.com/repos/$owner/$repo/releases/assets/$($a.id)" -Headers $headers | Out-Null
            }
        }
        Write-Host "  $leaf ($([Math]::Round((Get-Item $path).Length / 1MB, 1)) MB)"
        $uploadUrl = "https://uploads.github.com/repos/$owner/$repo/releases/$($release.id)/assets?name=$([Uri]::EscapeDataString($leaf))"
        Invoke-RestMethod -Method Post -Uri $uploadUrl -Headers $headers -InFile $path -ContentType 'application/octet-stream' | Out-Null
    }
    Write-Host ''
    if ($release.draft) {
        Write-Ok "Uploaded to the draft release v$version. Publish it on GitHub to make it live - $($release.html_url)"
    } else {
        Write-Ok "Uploaded to the release v${version}: $($release.html_url)"
    }
    Write-Host '  Installed apps (Tauri and Electron) update to it within a few hours, or menu > Check for updates.'
}

# ---------------------------------------------------------------------------
# actions
# ---------------------------------------------------------------------------

function Invoke-Setup {
    <# One time: Rust (rustup) and the Visual Studio C++ build tools, with winget. #>
    if (-not (Get-Command winget -ErrorAction SilentlyContinue)) {
        throw 'winget was not found. Install "App Installer" from the Microsoft Store, then run Setup again.'
    }
    if (Test-CppTools) {
        Write-Ok 'C++ build tools: already installed.'
    } else {
        Write-Step 'Installing the Visual Studio C++ build tools (about 2-4 GB, 10-20 minutes; Windows asks for permission)'
        & winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-source-agreements --accept-package-agreements `
            --override '--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended'
        if (-not (Test-CppTools)) { throw 'The C++ build tools did not install. Run Setup again, or install "Desktop development with C++" from https://visualstudio.microsoft.com/visual-cpp-build-tools/.' }
        Write-Ok 'C++ build tools installed.'
    }
    if (Find-Cargo) {
        Write-Ok 'Rust: already installed.'
    } else {
        Write-Step 'Installing Rust (rustup)'
        & winget install --id Rustlang.Rustup -e --accept-source-agreements --accept-package-agreements
        if (-not (Find-Cargo)) { throw 'Rust did not install. Run Setup again, or install it from https://rustup.rs.' }
        Write-Ok 'Rust installed.'
    }
    $rustup = Join-Path (Split-Path (Find-Cargo)) 'rustup.exe'
    if (Test-Path $rustup) { & $rustup default stable-msvc | Out-Null }
    Install-UiDeps
    Write-Ok 'Ready to build.'
}

function Invoke-Key {
    <# One time: the key pair installed apps use to trust updates (the same key as I1 Phone). #>
    if (Test-Path $KeyFile) {
        Write-Ok "The update key already exists: $KeyFile"
        Write-Host '  Keep a copy of it somewhere safe (and never in git): installed apps only accept updates signed with it.'
        return
    }
    Assert-Node
    New-Item -ItemType Directory -Force -Path (Split-Path $KeyFile) | Out-Null
    Invoke-Native 'Making the update key' { npx --yes $TauriCli signer generate --ci -w $KeyFile | Out-Null }
    Write-Ok "Made $KeyFile (private) and $KeyFile.pub (public)."
    Write-Warn2 '  Back up the private key now (e.g. a password manager or a USB stick). If it is lost,'
    Write-Warn2 '  installed apps cannot be updated any more and must be reinstalled by hand.'
}

function Invoke-Dev {
    Assert-BuildTools
    if ($Server) {
        $env:I1FRAME_SERVER = $Server
        Write-Host "Using $Server (development only)."
    }
    Install-UiDeps
    Write-Utf8 $PackagedCfg ((Get-Content $StandardCfg -Raw).Trim() + "`n")
    try {
        Invoke-Native 'Building the title bar and its screens' { npm run build:ui }
        Invoke-Native 'Starting the desktop app (close the window and quit from the tray, or press Ctrl+C here)' { npx --yes $TauriCli dev } -In $TauriDir
    } finally {
        Write-Utf8 $PackagedCfg "{}`n"
    }
}

function Invoke-Typecheck {
    Assert-BuildTools
    Install-UiDeps
    Invoke-Native 'Type check (title bar)' { npx tsc --noEmit -p tsconfig.web.json }
    Invoke-Native 'Compile check + unit tests' { cargo test --lib } -In $SrcTauri
}

function Invoke-Build {
    if (-not (Test-Path $StandardCfg)) { throw "Missing $StandardCfg (the standard installer's server). Set it in ImagineOne Studio: Desktop app > Installer config." }
    $cfgJson = (Get-Content $StandardCfg -Raw).Trim()
    $null = $cfgJson | ConvertFrom-Json   # must be valid JSON
    Write-Host "Standard installer, server settings: $cfgJson" -ForegroundColor Green
    Invoke-TauriBuild -DistDir (Join-Path $Here 'dist') -ConfigJson ($cfgJson + "`n")
    if ($Publish) { Invoke-PublishBuild }
}

function Invoke-CustomerBuild {
    $c = Get-CustomerProfile $Customer
    # Artwork: the customer's icon.ico / icon.png, made from logo.png when missing.
    $ico = Join-Path $c.Folder 'icon.ico'
    $png = Join-Path $c.Folder 'icon.png'
    if (-not (Test-Path $ico) -or -not (Test-Path $png)) {
        $logo = Join-Path $c.Folder 'logo.png'
        $make = Join-Path $Tools 'make-icons.js'
        if ((Test-Path $logo) -and (Test-Path $make)) {
            Assert-Node
            Invoke-Native 'Making icons from logo.png' { node $make $logo $c.Folder }
        } else {
            Write-Warn2 "  No icon.ico/icon.png or logo.png in $($c.Folder) - using the standard icon."
            $ico = ''; $png = ''
        }
    }
    $cfgJson = [ordered]@{ baseUrl = $c.BaseUrl; tenantSlug = $c.Slug } | ConvertTo-Json
    Write-Host ''
    Write-Host "Building '$($c.Name)' for $($c.BaseUrl) (company $($c.Slug))" -ForegroundColor Green
    Invoke-TauriBuild -DistDir (Join-Path $c.Folder 'dist') -ConfigJson ($cfgJson + "`n") -Name $c.Name -Slug $c.Slug `
        -Owner $c.Owner -Repo $c.Repo -Ico $ico -Png $png
    if ($Publish) { Invoke-PublishBuild }
}

function Invoke-Bump {
    Assert-Node
    $before = Get-Version
    Invoke-Native 'Raising the version' { npm version patch --no-git-tag-version | Out-Null }
    Write-Ok "Version $before -> $(Get-Version). Build again, then Publish, and installed apps update themselves."
}

function Show-Menu {
    while ($true) {
        Write-Host ''
        Write-Host '  ImagineOne desktop app' -ForegroundColor Cyan
        Write-Host "  version $(Get-Version)   $Here"
        Write-Host ''
        Write-Host '   1  Run it (debug build)'
        Write-Host '   2  Build the standard installer'
        Write-Host '   3  Build a customer installer'
        Write-Host '   4  Raise the version'
        Write-Host '   5  Publish the last build to GitHub (needs $env:GH_TOKEN)'
        Write-Host '   6  Type check + compile check + unit tests'
        Write-Host '   S  Setup: install the build tools (one time)'
        Write-Host '   K  Make the update signing key (one time)'
        Write-Host '   Q  Quit'
        $choice = (Read-Host '  Choose').Trim().ToUpper()
        try {
            switch ($choice) {
                '1' { $script:Server = (Read-Host '  Server for testing, e.g. https://app.imagineone.it (Enter to use resources\frame-config.json)').Trim(); Invoke-Dev }
                '2' { Invoke-Build }
                '3' { $script:Customer = (Read-Host '  Customer slug').Trim().ToLower(); Invoke-CustomerBuild }
                '4' { Invoke-Bump }
                '5' { $script:Customer = (Read-Host '  Customer slug (Enter for the standard build)').Trim().ToLower(); Invoke-PublishBuild }
                '6' { Invoke-Typecheck }
                'S' { Invoke-Setup }
                'K' { Invoke-Key }
                'Q' { return }
                default { Write-Warn2 '  Pick 1-6, S, K or Q.' }
            }
        } catch {
            Write-Host ''
            Write-Host "ERROR: $($_.Exception.Message)" -ForegroundColor Red
        }
    }
}

switch ($Action) {
    'Menu'      { Show-Menu }
    'Setup'     { Invoke-Setup }
    'Key'       { Invoke-Key }
    'Dev'       { Invoke-Dev }
    'Build'     { Invoke-Build }
    'Customer'  { Invoke-CustomerBuild }
    'Bump'      { Invoke-Bump }
    'Publish'   { Invoke-PublishBuild }
    'Typecheck' { Invoke-Typecheck }
}
