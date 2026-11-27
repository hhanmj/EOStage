# Sentinel-1 Burst CMR/UMM fixture provenance

Captured read-only from the public NASA CMR metadata API at
`2026-08-25T17:03Z`; no credentials, cookies, signed URLs, or science bytes were
used.

- Collection query:
  `https://cmr.earthdata.nasa.gov/search/collections.umm_json?provider=ASF&short_name=SENTINEL-1_BURSTS&page_size=10`
- Collection identity: concept ID `C2709161906-ASF`, native ID and short name
  `SENTINEL-1_BURSTS`, revision ID `15`, revision date
  `2026-05-20T20:16:58.761Z`, provider `ASF`.
- Granule query:
  `https://cmr.earthdata.nasa.gov/search/granules.umm_json?collection_concept_id=C2709161906-ASF&page_size=1&page_num=1`
- Response media contract: UMM-G `1.6.7`; `CMR-Hits: 163388001`;
  `CMR-Search-After: ["asf",1412314957182,3128067030]`.
- First authoritative granule: concept ID `G3128067030-ASF`, native ID
  `S1_301495_IW3_20141003T054235_VV_D5C8-BURST`, collection
  `C2709161906-ASF`.

The JSON fixture is a deterministic sanitized complete first-page raw granule
UMM response. It preserves `hits`/`took`, granule metadata, temporal/orbit
identity, every captured AdditionalAttribute in original order, the returned
seven-point GPolygon, collection reference, PGE version, exact RelatedUrls,
InputGranules, platform/instrument/look direction, ProviderDates, and
MetadataSpecification. Transport-only request IDs are intentionally omitted.

Only these case-sensitive aliases are authorized for Burst product mapping:
`BURST_ID_ABSOLUTE`, `BURST_ID_FULL`, `BURST_ID_RELATIVE`, `BURST_INDEX`,
`SUBSWATH_NAME`, `BYTE_LENGTH`, `PATH_NUMBER`, `POLARIZATION`,
`PROCESSING_LEVEL`, `PROCESSING_TYPE`, `ASCENDING_DESCENDING`, `AZIMUTH_TIME`,
`AZIMUTH_ANX_TIME`, and `SAMPLES_PER_BURST`.

The three `USE SERVICE API` links are distinct transport candidates. Task 3B
publishes only the exact GeoTIFF and XML assets. The SAFE ZIP is not an admitted
asset. The sole `VIEW RELATED INFORMATION` link is documentation, not browse;
preview is therefore explicitly absent.
