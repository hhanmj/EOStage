[CmdletBinding()]
param(
    [ValidateSet("x64", "x86")]
    [string]$Architecture = "x64",
    [switch]$Quiet
)

$ErrorActionPreference = "Stop"

function Find-FirstExistingFile([string[]]$Candidates) {
    foreach ($candidate in $Candidates) {
        if (-not [string]::IsNullOrWhiteSpace($candidate) -and
            (Test-Path -LiteralPath $candidate -PathType Leaf)) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }
    return $null
}

function Assert-Hdf5CmakeCacheGenerator([string]$RepoRoot, [string]$ExpectedGenerator) {
    $targetRoot = Join-Path $RepoRoot "target"
    if (-not (Test-Path -LiteralPath $targetRoot -PathType Container)) {
        return
    }

    foreach ($profile in @("debug", "release")) {
        $buildRoot = Join-Path $targetRoot ("{0}\build" -f $profile)
        if (-not (Test-Path -LiteralPath $buildRoot -PathType Container)) {
            continue
        }
        $packageRoots = @(Get-ChildItem -LiteralPath $buildRoot -Directory -Filter "hdf5-metno-src-*" -ErrorAction SilentlyContinue)
        foreach ($packageRoot in $packageRoots) {
            $cachePath = Join-Path $packageRoot.FullName "out\build\CMakeCache.txt"
            if (-not (Test-Path -LiteralPath $cachePath -PathType Leaf)) {
                continue
            }
            $cacheText = Get-Content -LiteralPath $cachePath -Raw
            $cacheMatch = [regex]::Match($cacheText, "(?m)^CMAKE_GENERATOR:INTERNAL=(?<generator>.+)$")
            if ($cacheMatch.Success) {
                $cachedGenerator = $cacheMatch.Groups["generator"].Value.Trim()
                if ($cachedGenerator -ne $ExpectedGenerator) {
                    throw ("hdf5-metno-src has a stale CMake cache generator '{0}' at {1}; " +
                        "the current process requires '{2}'. Refusing to reuse the cache. " +
                        "After confirming the path is this worktree's package cache, run " +
                        "'cargo clean -p hdf5-metno-src' and rerun the initialized build.") -f
                        $cachedGenerator, $cachePath, $ExpectedGenerator
                }
            }
        }
    }
}

$programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
$programFiles = [Environment]::GetEnvironmentVariable("ProgramFiles")
$vswhereCandidates = @()
if ($programFilesX86) {
    $vswhereCandidates += Join-Path $programFilesX86 "Microsoft Visual Studio\Installer\vswhere.exe"
}
if ($programFiles) {
    $vswhereCandidates += Join-Path $programFiles "Microsoft Visual Studio\Installer\vswhere.exe"
}
$vswhere = Find-FirstExistingFile $vswhereCandidates
if (-not $vswhere) {
    throw "vswhere.exe was not found."
}

$instancesJson = (& $vswhere -all -products '*' -format json | Out-String)
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($instancesJson)) {
    throw "vswhere could not enumerate Visual Studio instances."
}
$instances = @($instancesJson | ConvertFrom-Json)
$completeInstances = @(
    $instances | Where-Object {
        $_.isComplete -eq $true -and
        $_.isLaunchable -eq $true -and
        -not [string]::IsNullOrWhiteSpace($_.installationPath)
    }
)
if ($completeInstances.Count -eq 0) {
    throw "No complete and launchable Visual Studio instance was found."
}

$selected = $completeInstances | Sort-Object -Property @{ Expression = { if ($_.installationVersion.StartsWith("18.")) { 0 } else { 1 } } }, @{ Expression = { $_.installationVersion }; Descending = $true } | Select-Object -First 1
$vsInstallPath = [System.IO.Path]::GetFullPath([string]$selected.installationPath)
$devShell = Join-Path $vsInstallPath "Common7\Tools\Microsoft.VisualStudio.DevShell.dll"
if (-not (Test-Path -LiteralPath $devShell -PathType Leaf)) {
    throw "Visual Studio DevShell was not found: $devShell"
}

Import-Module -Name $devShell -Force
if (-not (Get-Command Enter-VsDevShell -ErrorAction SilentlyContinue)) {
    throw "Visual Studio DevShell does not export Enter-VsDevShell."
}
# DevShell names the 64-bit target amd64; the SDK library folders below use x64.
$devShellArch = if ($Architecture -eq "x64") { "amd64" } else { $Architecture }
Enter-VsDevShell -VsInstallPath $vsInstallPath -SkipAutomaticLocation -Arch $devShellArch | Out-Null

$env:HDF5_PLUGIN_PRELOAD = "::"

foreach ($toolName in @("cl.exe", "link.exe", "nmake.exe")) {
    $tool = @(Get-Command $toolName -CommandType Application -ErrorAction SilentlyContinue) | Where-Object { $_.Source -and $_.Source.StartsWith($vsInstallPath, [System.StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1
    if (-not $tool) {
        throw "The Visual Studio Developer Shell does not contain $toolName."
    }
    $toolPath = [System.IO.Path]::GetFullPath($tool.Source)
    if (-not $toolPath.StartsWith($vsInstallPath, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "$toolName was not resolved from the selected Visual Studio: $toolPath"
    }
}

$cmakeCandidates = @(Get-Command cmake.exe -CommandType Application -ErrorAction SilentlyContinue)
$cmake = $cmakeCandidates | Where-Object { $_.Source -and $_.Source.StartsWith($vsInstallPath, [System.StringComparison]::OrdinalIgnoreCase) } | Select-Object -First 1
if (-not $cmake) {
    $cmake = $cmakeCandidates | Select-Object -First 1
}
if (-not $cmake) {
    $vsCmake = Join-Path $vsInstallPath "Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin\cmake.exe"
    if (Test-Path -LiteralPath $vsCmake -PathType Leaf) {
        $vsCmakeDir = Split-Path -Parent $vsCmake
        $env:Path = $vsCmakeDir + ";" + $env:Path
        $cmake = Get-Command cmake.exe -CommandType Application -ErrorAction SilentlyContinue
    }
}
if (-not $cmake) {
    throw "cmake.exe was not found."
}
$cmakePath = [System.IO.Path]::GetFullPath($cmake.Source)
$cmakeDir = Split-Path -Parent $cmakePath
if (-not $env:Path.StartsWith($cmakeDir, [System.StringComparison]::OrdinalIgnoreCase)) {
    $env:Path = $cmakeDir + ";" + $env:Path
}
$cmakeVersion = ((& $cmakePath --version | Select-Object -First 1) -join "").Trim()
$cmakeHelp = (& $cmakePath --help | Out-String)
$supportsNMake = $cmakeHelp -match "NMake Makefiles"

if ($supportsNMake) {
    $ambientGenerator = [string]$env:CMAKE_GENERATOR
    $ambientGeneratorInstance = [string]$env:CMAKE_GENERATOR_INSTANCE
    $ambientGeneratorPlatform = [string]$env:CMAKE_GENERATOR_PLATFORM
    $ambientGeneratorToolset = [string]$env:CMAKE_GENERATOR_TOOLSET
    $env:CMAKE_GENERATOR = "NMake Makefiles"
    $env:CMAKE_GENERATOR_PLATFORM = $null
    $env:CMAKE_GENERATOR_INSTANCE = $null
    $env:CMAKE_GENERATOR_TOOLSET = $null
    $selectedGenerator = $env:CMAKE_GENERATOR
} else {
    throw "The selected CMake does not support the required NMake Makefiles generator: $cmakePath"
}

Assert-Hdf5CmakeCacheGenerator -RepoRoot (Split-Path -Parent $PSScriptRoot) -ExpectedGenerator $selectedGenerator

$sdkRoot = [string]$env:WindowsSdkDir
$sdkVersion = [string]$env:WindowsSDKVersion
$sdkVersion = $sdkVersion.TrimEnd([char]92)
if ([string]::IsNullOrWhiteSpace($sdkRoot) -or [string]::IsNullOrWhiteSpace($sdkVersion)) {
    throw "The Visual Studio Developer Shell did not provide a Windows SDK."
}
$sdkLibRoot = Join-Path $sdkRoot ("Lib\" + $sdkVersion)
$kernel32 = Join-Path $sdkLibRoot ("um\" + $Architecture + "\kernel32.lib")
$ucrt = Join-Path $sdkLibRoot ("ucrt\" + $Architecture + "\ucrt.lib")
if (-not (Test-Path -LiteralPath $kernel32 -PathType Leaf)) {
    throw "Windows SDK kernel32.lib was not found: $kernel32"
}
if (-not (Test-Path -LiteralPath $ucrt -PathType Leaf)) {
    throw "Windows SDK ucrt.lib was not found: $ucrt"
}

if (-not $Quiet) {
    Write-Host ("Windows native toolchain initialized: {0} {1}" -f $selected.displayName, $selected.installationVersion)
    Write-Host ("VS path: {0}" -f $vsInstallPath)
    Write-Host ("MSVC: cl/link/nmake from {0}" -f $vsInstallPath)
    Write-Host ("SDK: {0} ({1})" -f $sdkRoot, $sdkVersion)
    Write-Host ("CMake: {0} ({1}); generator: {2}" -f $cmakePath, $cmakeVersion, $selectedGenerator)
    if (-not [string]::IsNullOrWhiteSpace($ambientGenerator)) {
        Write-Host ("Ignored ambient CMAKE_GENERATOR: {0}" -f $ambientGenerator) -ForegroundColor Yellow
    }
    if (-not [string]::IsNullOrWhiteSpace($ambientGeneratorInstance) -or
        -not [string]::IsNullOrWhiteSpace($ambientGeneratorPlatform) -or
        -not [string]::IsNullOrWhiteSpace($ambientGeneratorToolset)) {
        Write-Host "Ignored ambient CMake generator instance/platform/toolset values." -ForegroundColor Yellow
    }
    Write-Host "HDF5_PLUGIN_PRELOAD=::"
}
