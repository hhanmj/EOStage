#!/usr/bin/env bash
# Prepare the current shell for Linux native Cargo/CMake/HDF5 builds and tests.
# Source it: `. scripts/initialize_linux_native_build.sh`
#
# Linux counterpart of scripts/initialize_windows_native_build.ps1 for the HDF5
# process environment. The packaged host establishes the same values itself in
# insar_download::establish_hdf5_process_isolation(); test binaries that never
# call it need them from the environment.
#  - HDF5_PLUGIN_PRELOAD=:: starts HDF5 with dynamic plugin loading disabled.
#  - HDF5_USE_FILE_LOCKING=FALSE leaves exclusive access to the application's
#    own flock on writer/promotion handles (Windows HDF5 does not lock).

for tool in cc c++ cmake pkg-config; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "initialize_linux_native_build: missing required tool: $tool" >&2
        return 1 2>/dev/null || exit 1
    fi
done

export HDF5_PLUGIN_PRELOAD="::"
export HDF5_USE_FILE_LOCKING="FALSE"
unset HDF5_PLUGIN_PATH HDF5_VOL_CONNECTOR HDF5_DRIVER HDF5_DRIVER_CONFIG

echo "initialize_linux_native_build: cmake $(cmake --version | head -n1 | awk '{print $3}'), HDF5 env prepared"
