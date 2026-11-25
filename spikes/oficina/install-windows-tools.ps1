# What spikes\oficina\run-windows.ps1 needs on a Windows 11 x64 machine, installed in one command:
#
#   powershell -ExecutionPolicy Bypass -File spikes\oficina\install-windows-tools.ps1
#
# Run it elevated (Visual Studio's installer and Git's install for the machine); it needs no clone,
# so it can be run before one. What is already installed is kept. Every piece comes from its
# publisher and is checked before it runs: against the SHA-256 written here (the publisher's sums,
# or the digest GitHub keeps for a release's asset), against the sum rustup publishes beside its
# installer, or, for Visual Studio's bootstrapper, which has no published sum, against Microsoft's
# signature. Qt comes through aqtinstall, which checks each archive against Qt's own sums. The
# versions are those docs/oficina-plan.md lists ("The runs elsewhere"). Its log:
# %USERPROFILE%\.cache\oficina-spike\install-tools.txt.

param(
    [string]$Opt = "$env:USERPROFILE\.local\opt",
    [string]$QtRoot = "C:\Qt"
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$Cache = Join-Path $env:USERPROFILE ".cache\oficina-spike"
$Downloads = Join-Path $Cache "downloads"
$Bin = Join-Path $Opt "bin"
New-Item -ItemType Directory -Force $Downloads, $Bin | Out-Null
Start-Transcript -Path (Join-Path $Cache "install-tools.txt") -Append | Out-Null

# A file from its publisher, kept in the downloads folder, its SHA-256 checked when one is given.
function Fetch([string]$Url, [string]$Sha256) {
    $file = Join-Path $Downloads ([IO.Path]::GetFileName(([Uri]$Url).AbsolutePath))
    if (-not (Test-Path $file)) {
        & curl.exe -fsSL --retry 3 -o "$file.part" $Url
        if ($LASTEXITCODE -ne 0) { throw "download failed (curl exit $LASTEXITCODE): $Url" }
        Move-Item -Force "$file.part" $file
    }
    if ($Sha256) {
        $got = (Get-FileHash -Algorithm SHA256 $file).Hash.ToLower()
        if ($got -ne $Sha256.ToLower()) { Remove-Item $file; throw "SHA-256 of $file is $got, not $Sha256" }
        Write-Host "checked $([IO.Path]::GetFileName($file)): sha256 $got"
    }
    return $file
}

# A folder on the user's PATH, for this process and for every later logon.
function AddPath([string]$Dir) {
    $user = [Environment]::GetEnvironmentVariable("Path", "User")
    $parts = @(if ($user) { $user -split ";" | Where-Object { $_ } })
    if ($parts -notcontains $Dir) { [Environment]::SetEnvironmentVariable("Path", (($parts + $Dir) -join ";"), "User") }
    if (($env:Path -split ";") -notcontains $Dir) { $env:Path = "$Dir;$env:Path" }
}

function Run([string]$Exe, [string]$Arguments, [int[]]$Ok = @(0)) {
    $p = Start-Process -FilePath $Exe -ArgumentList $Arguments -Wait -PassThru -NoNewWindow
    if ($Ok -notcontains $p.ExitCode) { throw "$([IO.Path]::GetFileName($Exe)) exited $($p.ExitCode)" }
}

# Git for Windows 2.56.1, machine-wide (Git\cmd on the machine's PATH).
$git = Join-Path $env:ProgramFiles "Git\cmd\git.exe"
if (-not (Test-Path $git)) {
    $f = Fetch "https://github.com/git-for-windows/git/releases/download/v2.56.1.windows.1/Git-2.56.1-64-bit.exe" "c330c72e10b408cc39994bfefdb63542b0d93d21699325c3e0bd874af8a5a559"
    Run $f "/VERYSILENT /NORESTART /NOCANCEL /SP- /SUPPRESSMSGBOXES"
}
AddPath (Split-Path $git)
& $git config --global core.longpaths true

# Visual Studio 2022 Build Tools (17.x, the latest), workload "Desktop development with C++" with its
# recommended parts: MSVC v143 for x64, a Windows 11 SDK, CMake and Ninja for Visual Studio.
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$vs = if (Test-Path $vswhere) { & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath } else { $null }
if (-not $vs) {
    $f = Fetch "https://aka.ms/vs/17/release/vs_BuildTools.exe" ""
    $sig = Get-AuthenticodeSignature $f
    if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notmatch "O=Microsoft Corporation") {
        throw "vs_BuildTools.exe is not signed by Microsoft: $($sig.Status), $($sig.SignerCertificate.Subject)"
    }
    Write-Host "checked vs_BuildTools.exe: signed by $($sig.SignerCertificate.Subject)"
    Run $f "--quiet --wait --norestart --nocache --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" @(0, 3010)
}

# Rust 1.100.0 through rustup, host x86_64-pc-windows-msvc, with rust-src and rust-analyzer.
$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (-not (Test-Path (Join-Path $cargoBin "rustup.exe"))) {
    $base = "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe"
    $sum = ((& curl.exe -fsSL "$base.sha256") -split "\s+")[0]
    if ($LASTEXITCODE -ne 0 -or $sum -notmatch "^[0-9a-f]{64}$") { throw "rustup-init's published sum not read: $sum" }
    $f = Fetch $base $sum
    Run $f "-y --default-toolchain 1.100.0 --profile minimal --component rust-src,rust-analyzer"
}
AddPath $cargoBin
& rustup toolchain install 1.100.0 --profile minimal --component rust-src,rust-analyzer
& rustup default 1.100.0

# cargo-nextest 0.9.152.
if (-not (Test-Path (Join-Path $cargoBin "cargo-nextest.exe"))) {
    $f = Fetch "https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-0.9.152/cargo-nextest-0.9.152-x86_64-pc-windows-msvc.zip" "cc6bf4b6a7ac18e462cd6c9bae67e26921b888249c2988c6acabf3df387ed9d6"
    Expand-Archive -Force $f $cargoBin
}

# CMake 4.5.1 and Ninja 1.13.2, ahead of Visual Studio's own on PATH.
$cmake = Join-Path $Opt "cmake-4.5.1-windows-x86_64"
if (-not (Test-Path "$cmake\bin\cmake.exe")) {
    Expand-Archive -Force (Fetch "https://github.com/Kitware/CMake/releases/download/v4.5.1/cmake-4.5.1-windows-x86_64.zip" "bfe6eadb245dc8213e0d3feea316b520cb2aae92f5667ffd76059ba6b05c430c") $Opt
}
AddPath "$cmake\bin"
if (-not (Test-Path "$Bin\ninja.exe")) {
    Expand-Archive -Force (Fetch "https://github.com/ninja-build/ninja/releases/download/v1.13.2/ninja-win.zip" "07fc8261b42b20e71d1720b39068c2e14ffcee6396b76fb7a795fb460b78dc65") $Bin
}
AddPath $Bin

# Node.js 26.13.0 with its npm.
$node = Join-Path $Opt "node-v26.13.0-win-x64"
if (-not (Test-Path "$node\node.exe")) {
    Expand-Archive -Force (Fetch "https://nodejs.org/dist/v26.13.0/node-v26.13.0-win-x64.zip" "dcc60d59a0746999f99064a01c020445d54d84228c691cdbf25b49ac3b59150a") $Opt
}
AddPath $node

# Qt 6.12.0 for MSVC 2022 64-bit with the spike's modules, through aqtinstall's master at the commit
# after its Qt 6.12 support (its last release, 3.3.0, cannot read Qt 6.12's Windows repository), run
# by uv 0.13.9, which fetches the Python it runs on.
if (-not (Test-Path "$Bin\uv.exe")) {
    Expand-Archive -Force (Fetch "https://github.com/astral-sh/uv/releases/download/0.13.9/uv-x86_64-pc-windows-msvc.zip" "edd61f7f5728a56cc62455386661860bdfc7bab65872f39b0c25bdcc1d8e1beb") $Bin
}
$qt = Join-Path $QtRoot "6.12.0\msvc2022_64"
if (-not (Test-Path "$qt\bin\QtWebEngineProcess.exe")) {
    & "$Bin\uvx.exe" --from "git+https://github.com/miurahr/aqtinstall@4c759852b1108cc801d8e24979136b895b5155d4" aqt install-qt windows desktop 6.12.0 win64_msvc2022_64 -O $QtRoot -m qtquick3d qtshadertools qtwebchannel qtpositioning qtwebengine
    if ($LASTEXITCODE -ne 0) { throw "aqt install-qt exited $LASTEXITCODE" }
}

# Oficina's window built by any cargo run of the suite (docs/questions.md Q370, ruled: built by
# default): cxx-qt finds Qt through `QMAKE`, and a test of Oficina's package, which links the
# window, finds Qt's DLLs and the C++ libraries' (KDDockWidgets) on `PATH`.
[Environment]::SetEnvironmentVariable("QMAKE", "$qt\bin\qmake.exe", "User")
$env:QMAKE = "$qt\bin\qmake.exe"
AddPath "$qt\bin"
AddPath (Join-Path $env:USERPROFILE ".local\opt\oficina-deps\bin")

# What is installed now, as the run will find it.
$vs = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property catalog_productDisplayVersion
Write-Host "Visual Studio Build Tools $vs"
& git --version
& rustc --version
& cargo nextest --version
& cmake --version | Select-Object -First 1
Write-Host "ninja $(& ninja --version)"
Write-Host "node $(& node --version), npm $(& "$node\npm.cmd" --version)"
Write-Host "Qt $(& "$qt\bin\qmake.exe" -query QT_VERSION) in $qt"
Stop-Transcript | Out-Null
