# HDF5 Chunk Range Planning Contract

1. Fetch metadata prefix and parse chunk B-tree addresses.
2. Project AOI polygon into radar slant-range / zero-Doppler time coordinates via geolocation grid cubes.
3. Compute minimal contiguous byte ranges with bounded request budget.
4. Reassemble valid HDF5 and verify post-write science chunk digests bit-for-bit.
