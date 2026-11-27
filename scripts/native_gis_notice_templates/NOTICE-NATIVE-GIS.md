# EOStage native GIS runtime notice

This product-owned runtime is a Windows x64 native GDAL/PROJ import closure.
Its roots are `gdal.dll` and `proj_9.dll`; the exact DLL closure, package
archive URLs, package archive SHA-256 values, per-DLL SHA-256 values, and byte
counts are recorded in `BINARY-PROVENANCE.json`.

The staged runtime is deliberately narrower than a general GIS environment:
it contains no Python, Conda runtime, QGIS, OSGeo4W, `.pyd`, executable, GDAL
plugin directory, or GDAL plugin DLL.  The product loader uses the private
runtime directory and Windows DLL-directory loading; it does not use the
process `PATH` or a host GDAL/PROJ installation.

The reviewed raster capability is exactly `GTiff` plus `ENVI`.  `share/proj`
contains the product-owned `proj.db` and required PROJ data; `share/gdal`
contains the GDAL data required by the loaded core.  The runtime manifest
records `proj_network = disabled`, and the Rust loader additionally disables
PROJ networking on every created PROJ context.

The package license texts and notices are copied under this runtime's
`licenses/` directory.  The package license inventory in
`BINARY-PROVENANCE.json` is checked by `build_native_gis_runtime.ps1` before
staging; a missing notice or missing package license fails the build closed.

The bundled EGM96 and EGM2008 grids are separate product resources.  Their
source, CRS contract, coverage, resolution, license, admission decision, byte
count, and SHA-256 are recorded in `vertical-grids/manifest.json`.
