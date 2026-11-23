# Oficina's own-shell spike on Windows, in one command; its result in one file.
#
#   powershell -ExecutionPolicy Bypass -File spikes\oficina\run-windows.ps1 -Expect software -ExpectAdapter "Microsoft Basic Render"
#
# Run at a clone of the repository on the Windows machine, from any folder. It builds what the spike
# needs and has not got (KDDockWidgets, Spix and AnyRPC as on Linux, Monaco from npm, supervisao's
# host), runs the spike's suite on Qt's `offscreen` platform and on `windows`, then the release
# timings, and writes every step's output and exit code to spikes\oficina\target\windows-run.txt,
# whose last lines are the verdicts. A step that fails is written and the run goes on.
#
# What must be installed first is listed in docs/oficina-plan.md ("Windows", in the spike's
# section). -Expect names what draws this machine: `software` (no GPU: WARP) or `gpu`;
# -ExpectAdapter a word of the adapter's name. Windows PowerShell 5.1 is enough.

param(
    [string]$Qt = "C:\Qt\6.12.0\msvc2022_64",
    [string]$Deps = "$env:USERPROFILE\.local\opt\oficina-deps",
    [ValidateSet("", "software", "gpu")][string]$Expect = "",
    [string]$ExpectAdapter = ""
)

$ErrorActionPreference = "Continue"
$Spike = $PSScriptRoot
$Root = (Resolve-Path (Join-Path $Spike "..\..")).Path
$Result = Join-Path $Spike "target\windows-run.txt"
$Sources = Join-Path $env:USERPROFILE ".cache\oficina-spike\src"
New-Item -ItemType Directory -Force (Split-Path $Result), $Sources | Out-Null
Set-Content -Path $Result -Encoding utf8 -Value "Oficina spike, Windows run, $(Get-Date -Format o)"
$Verdicts = New-Object System.Collections.Generic.List[string]

function Log([string]$Text) {
    Add-Content -Path $Result -Encoding utf8 -Value $Text
    Write-Host $Text
}

# One step: a command run by cmd (its stderr merged, so PowerShell 5.1 turns no line into an error),
# in a folder, its output and exit code written to the result.
function Step([string]$Name, [string]$Command, [string]$Folder = $Root) {
    Log ""
    Log "== $Name"
    Log "> $Command   (in $Folder)"
    Push-Location $Folder
    $out = cmd /c "$Command 2>&1"
    $code = $LASTEXITCODE
    Pop-Location
    if ($out) { Add-Content -Path $Result -Encoding utf8 -Value $out }
    Log "exit $code"
    $Verdicts.Add(("{0,-52} exit {1}" -f $Name, $code))
    return $code
}

# The machine, as the result will be read.
Log "Windows: $((Get-CimInstance Win32_OperatingSystem).Caption) $((Get-CimInstance Win32_OperatingSystem).Version)"
Log "Processor: $((Get-CimInstance Win32_Processor | Select-Object -First 1).Name), $((Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors) logical, $([math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB, 1)) GB"
foreach ($v in Get-CimInstance Win32_VideoController) { Log "Display adapter: $($v.Name), driver $($v.DriverVersion)" }
Log "Qt: $Qt   dependencies: $Deps   expected drawing: '$Expect' '$ExpectAdapter'"

# Microsoft's C++ tools, entered as their developer shell enters them.
$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$vs = if (Test-Path $vswhere) { & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath } else { $null }
if (-not $vs) {
    Log "No Visual Studio with the C++ tools (vswhere found none): nothing can be built."
    exit 1
}
Import-Module (Join-Path $vs "Common7\Tools\Microsoft.VisualStudio.DevShell.dll")
Enter-VsDevShell -VsInstallPath $vs -SkipAutomaticLocation -DevCmdArguments "-arch=x64 -host_arch=x64" | Out-Null
$env:QMAKE = Join-Path $Qt "bin\qmake.exe"
$env:OFICINA_DEPS = $Deps
$env:PATH = "$(Join-Path $Qt 'bin');$(Join-Path $Deps 'bin');$env:PATH"
$env:CMAKE_GENERATOR = "Ninja"
$jobs = [math]::Max(1, [math]::Min(4, (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors))
foreach ($tool in "cl", "cmake", "ninja", "git", "cargo", "cargo-nextest", "npm", "rustc") {
    $found = Get-Command $tool -ErrorAction SilentlyContinue
    Log ("{0,-14} {1}" -f $tool, $(if ($found) { $found.Source } else { "MISSING" }))
}
Step "versions" "rustc --version && cargo nextest --version && cmake --version && ninja --version && node --version && npm --version && `"$env:QMAKE`" -query QT_VERSION" | Out-Null

# The dependencies, built as on Linux when missing: AnyRPC (master b1949b3, static), Spix 0.14
# (commit 7ab2831, static), KDDockWidgets 2.5.1 (its Qt Quick side, shared).
$common = "-G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=`"$Deps`" -DCMAKE_PREFIX_PATH=`"$Qt;$Deps`""
if (-not (Test-Path (Join-Path $Deps "lib\anyrpc.lib"))) {
    $src = Join-Path $Sources "anyrpc"
    if (-not (Test-Path $src)) { Step "AnyRPC: fetch" "git clone -q https://github.com/sgieseking/anyrpc.git `"$src`" && git -C `"$src`" checkout -q b1949b3d4084" | Out-Null }
    Step "AnyRPC: build" "cmake -S `"$src`" -B `"$src\build`" $common -DANYRPC_LIB_BUILD_SHARED=OFF -DBUILD_EXAMPLES=OFF -DBUILD_TESTS=OFF -DBUILD_WITH_ADDRESS_SANITIZE=OFF -DBUILD_PROTOCOL_MESSAGEPACK=OFF -DCMAKE_POLICY_VERSION_MINIMUM=3.5 && cmake --build `"$src\build`" -j $jobs && cmake --install `"$src\build`"" | Out-Null
}
if (-not (Test-Path (Join-Path $Deps "lib\SpixCore.lib"))) {
    $src = Join-Path $Sources "spix"
    if (-not (Test-Path $src)) { Step "Spix: fetch" "git clone -q https://github.com/faaxm/spix.git `"$src`" && git -C `"$src`" checkout -q 7ab2831" | Out-Null }
    Step "Spix: build" "cmake -S `"$src`" -B `"$src\build`" $common -DSPIX_QT_MAJOR=6 -DSPIX_BUILD_QTQUICK=ON -DSPIX_BUILD_QTWIDGETS=OFF -DSPIX_BUILD_EXAMPLES=OFF -DSPIX_BUILD_TESTS=OFF -DBUILD_SHARED_LIBS=OFF && cmake --build `"$src\build`" -j $jobs && cmake --install `"$src\build`"" | Out-Null
}
if (-not (Test-Path (Join-Path $Deps "lib\kddockwidgets-qt6.lib"))) {
    $src = Join-Path $Sources "kddockwidgets"
    if (-not (Test-Path $src)) { Step "KDDockWidgets: fetch" "git clone -q --depth 1 --branch v2.5.1 https://github.com/KDAB/KDDockWidgets.git `"$src`"" | Out-Null }
    Step "KDDockWidgets: build" "cmake -S `"$src`" -B `"$src\build`" $common -DKDDockWidgets_QT6=ON -DKDDockWidgets_FRONTENDS=qtquick -DKDDockWidgets_EXAMPLES=OFF -DKDDockWidgets_TESTS=OFF && cmake --build `"$src\build`" -j $jobs && cmake --install `"$src\build`"" | Out-Null
}

# Monaco, as its lockfile names it (npm checks each package's integrity), and supervisao's host.
Step "Monaco: npm ci" "npm ci --no-audit --no-fund" (Join-Path $Spike "shell\web") | Out-Null
Step "supervisao's host: build" "cargo build -p veredas-supervisao --bin veredas-supervisao" | Out-Null

# The suite: built once, then on `offscreen` and on `windows`, one test at a time, what each test
# prints kept (timings, the adapter found).
$env:TMPDIR = $null
Step "spike: build the tests" "cargo nextest run --workspace --no-run" $Spike | Out-Null
$env:OFICINA_TEST_PLATFORM = "offscreen"
Step "spike: suite on offscreen" "cargo nextest run --workspace --no-fail-fast --no-capture" $Spike | Out-Null
$env:OFICINA_TEST_PLATFORM = "windows"
if ($Expect) { $env:OFICINA_EXPECT_DRAWING = $Expect }
if ($ExpectAdapter) { $env:OFICINA_EXPECT_ADAPTER = $ExpectAdapter }
Step "spike: suite on windows" "cargo nextest run --workspace --no-fail-fast --no-capture" $Spike | Out-Null
Step "spike: point 7 twenty times on offscreen" "set OFICINA_TEST_PLATFORM=offscreen&& cargo nextest run -p oficina-spike-shell --test workbench --stress-count 20" $Spike | Out-Null
Step "spike: release timings on windows" "cargo nextest run --release -p oficina-spike-shell --no-fail-fast --no-capture -E `"test(under_a_second) | test(scrolled) | test(memory)`"" $Spike | Out-Null

Log ""
Log "== verdicts"
foreach ($v in $Verdicts) { Log $v }
