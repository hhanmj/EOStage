param(
    [string]$ExpectedRustVersion = "",
    [switch]$RequirePluginPreload
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
. (Join-Path $repoRoot "scripts\initialize_windows_native_build.ps1") -Quiet
$env:CARGO_NET_OFFLINE = "true"

function Invoke-VersionCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Command,
        [Parameter(Mandatory = $true)][string[]]$Arguments
    )

    $resolved = @(Get-Command $Command -ErrorAction SilentlyContinue) | Select-Object -First 1
    if (-not $resolved) {
        throw "$Command is required for the reviewed NISAR HDF5 build"
    }
    $output = & $resolved.Source @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed while verifying NISAR HDF5 build inputs"
    }
    return @($output)
}

function Assert-LockedPackage {
    param(
        [Parameter(Mandatory = $true)][string]$LockText,
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$Version
    )

    $escapedName = [regex]::Escape($Name)
    $pattern = "(?ms)^\[\[package\]\]\r?\nname = `"$escapedName`"\r?\nversion = `"(?<version>[^`"]+)`"(?:\r?\n|$)"
    $matches = [regex]::Matches($LockText, $pattern)
    if ($matches.Count -ne 1 -or $matches[0].Groups["version"].Value -ne $Version) {
        throw "Cargo.lock must pin exactly one $Name package at $Version for the reviewed NISAR HDF5 build"
    }
}

foreach ($name in @("HDF5_DIR", "HDF5_VERSION")) {
    $value = [Environment]::GetEnvironmentVariable($name, "Process")
    if (-not [string]::IsNullOrWhiteSpace($value)) {
        throw "$name must be unset or empty; prebuilt or alternate HDF5 inputs are not reviewed"
    }
}

if ($RequirePluginPreload) {
    $pluginPreload = [Environment]::GetEnvironmentVariable("HDF5_PLUGIN_PRELOAD", "Process")
    if ($pluginPreload -ne "::") {
        throw "HDF5_PLUGIN_PRELOAD must be exactly :: for NISAR validator tests"
    }
}

$rustLines = @(Invoke-VersionCommand -Command "rustc" -Arguments @("-vV"))
$rustText = $rustLines -join "`n"
$rustMatch = [regex]::Match($rustText, "(?m)^rustc (?<version>\d+\.\d+\.\d+) ")
if (-not $rustMatch.Success) {
    throw "Unable to parse rustc -vV for the NISAR HDF5 build"
}
$rustVersion = [version]$rustMatch.Groups["version"].Value
$minimumRustVersion = [version]"1.95.0"
if ($rustVersion -lt $minimumRustVersion) {
    throw "The locked insar-download production dependency graph requires rustc 1.95.0 or newer"
}
if (-not [string]::IsNullOrWhiteSpace($ExpectedRustVersion) -and
    $rustVersion -ne [version]$ExpectedRustVersion) {
    throw "Expected rustc $ExpectedRustVersion for the MSRV gate, found $rustVersion"
}
$rustHostMatch = [regex]::Match($rustText, "(?m)^host: (?<host>\S+)$")
if (-not $rustHostMatch.Success -or
    $rustHostMatch.Groups["host"].Value -ne "x86_64-pc-windows-msvc") {
    throw "The reviewed NISAR HDF5 Windows build requires rustc host x86_64-pc-windows-msvc"
}

$cmakeLines = @(Invoke-VersionCommand -Command "cmake" -Arguments @("--version"))
$cmakeLine = ([string]$cmakeLines[0]).Trim()
$reviewedCmakeBaseline = "cmake version 3.31.6"
$cmakeMatch = [regex]::Match($cmakeLine, "^cmake version (?<version>\d+\.\d+\.\d+)")
if (-not $cmakeMatch.Success -or [version]$cmakeMatch.Groups["version"].Value -lt [version]"3.31.6") {
    throw "The reviewed static HDF5 build requires $reviewedCmakeBaseline or newer; found: $cmakeLine"
}
$generator = [Environment]::GetEnvironmentVariable("CMAKE_GENERATOR", "Process")
if ($generator -ne "NMake Makefiles") {
    throw "The reviewed static HDF5 build requires the process-local NMake Makefiles generator selected by initialize_windows_native_build.ps1; found: $generator"
}
foreach ($generatorVariable in @("CMAKE_GENERATOR_INSTANCE", "CMAKE_GENERATOR_PLATFORM", "CMAKE_GENERATOR_TOOLSET")) {
    $generatorValue = [Environment]::GetEnvironmentVariable($generatorVariable, "Process")
    if (-not [string]::IsNullOrWhiteSpace($generatorValue)) {
        throw "$generatorVariable must be empty; generator selection must not inherit machine-global CMake state"
    }
}

$manifestPath = Join-Path $repoRoot "crates\insar-download\Cargo.toml"
$lockPath = Join-Path $repoRoot "Cargo.lock"
$manifestText = Get-Content -LiteralPath $manifestPath -Raw
$requiredDependency = 'hdf5 = { package = "hdf5-metno", version = "=0.12.6", features = ["static", "zlib"] }'
$requiredSysDependency = 'hdf5-sys = { package = "hdf5-metno-sys", version = "=0.12.1", features = ["hl"] }'
if (-not $manifestText.Contains('rust-version = "1.95.0"') -or
    -not $manifestText.Contains($requiredDependency) -or
    -not $manifestText.Contains($requiredSysDependency)) {
    throw "insar-download must retain the reviewed MSRV and exact static HDF5 dependency declarations"
}

$lockText = Get-Content -LiteralPath $lockPath -Raw
Assert-LockedPackage -LockText $lockText -Name "hdf5-metno" -Version "0.12.6"
Assert-LockedPackage -LockText $lockText -Name "hdf5-metno-sys" -Version "0.12.1"
Assert-LockedPackage -LockText $lockText -Name "hdf5-metno-derive" -Version "0.10.2"
Assert-LockedPackage -LockText $lockText -Name "hdf5-metno-src" -Version "0.10.2"
Assert-LockedPackage -LockText $lockText -Name "hdf5-metno-types" -Version "0.11.1"
Assert-LockedPackage -LockText $lockText -Name "pastey" -Version "0.2.3"
Assert-LockedPackage -LockText $lockText -Name "libz-sys" -Version "1.1.29"
if ($lockText -match '(?ms)^\[\[package\]\]\r?\nname = "paste"\r?\n') {
    throw "Cargo.lock must not reintroduce the unmaintained paste package"
}

$hdf5TreeLines = @(Invoke-VersionCommand -Command "cargo" -Arguments @(
    "tree", "--locked", "-p", "hdf5-metno@0.12.6", "--edges", "normal", "--prefix", "none"
))
$hdf5TreeText = $hdf5TreeLines -join "`n"
foreach ($requiredNode in @(
    "hdf5-metno v0.12.6",
    "hdf5-metno-sys v0.12.1",
    "hdf5-metno-derive v0.10.2",
    "hdf5-metno-src v0.10.2",
    "hdf5-metno-types v0.11.1",
    "pastey v0.2.3",
    "libz-sys v1.1.29"
)) {
    $nodePattern = "(?m)^" + [regex]::Escape($requiredNode) + "(?: |$)"
    if ($hdf5TreeText -notmatch $nodePattern) {
        throw "The resolved hdf5-metno graph is missing reviewed node $requiredNode"
    }
}

Write-Output "NISAR HDF5 build inputs verified: rustc $rustVersion x86_64-pc-windows-msvc, $cmakeLine, generator $generator, static HDF5 lock closure and resolved graph."
