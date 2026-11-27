#!/usr/bin/env bash
# Prepare the current shell for macOS native Cargo/CMake/HDF5 builds and tests.
# Source it: `. scripts/initialize_macos_native_build.sh`
#
# macOS counterpart of scripts/initialize_windows_native_build.ps1 and
# scripts/initialize_linux_native_build.sh for the HDF5 process environment.
#  - HDF5_PLUGIN_PRELOAD=:: starts HDF5 with dynamic plugin loading disabled.
#  - HDF5_USE_FILE_LOCKING=FALSE leaves exclusive access to the application's
#    own flock on writer/promotion handles.
#  - Scrubs ambient CMAKE_GENERATOR* variables so hdf5-metno-src uses a clean
#    process-local Unix Makefiles / Ninja configuration.

for tool in cc c++ cmake; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "initialize_macos_native_build: missing required tool: $tool (install Xcode Command Line Tools or Homebrew cmake)" >&2
        return 1 2>/dev/null || exit 1
    fi
done

unset CMAKE_GENERATOR CMAKE_GENERATOR_INSTANCE CMAKE_GENERATOR_PLATFORM CMAKE_GENERATOR_TOOLSET
export HDF5_PLUGIN_PRELOAD="::"
export HDF5_USE_FILE_LOCKING="FALSE"
unset HDF5_PLUGIN_PATH HDF5_VOL_CONNECTOR HDF5_DRIVER HDF5_DRIVER_CONFIG

echo "initialize_macos_native_build: $(uname -s)/$(uname -m), cmake $(cmake --version | head -n1 | awk '{print $3}'), HDF5 env prepared"
