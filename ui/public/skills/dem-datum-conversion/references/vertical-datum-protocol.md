# Vertical Datum Conversion Protocol for InSAR

## Standard Datum Table

| DEM Product | Horizontal CRS | Source Vertical Datum | Required Geoid Grid |
|---|---|---|---|
| Copernicus GLO-30 / GLO-90 | EPSG:4326 | EGM2008 (`EPSG:3855`) | `us_nga_egm08_25.tif` |
| SRTM GL1 / NASADEM | EPSG:4326 | EGM96 (`EPSG:5773`) | `us_nga_egm96_15.tif` |
| NISAR Static DEM | EPSG:4326 | WGS84 Ellipsoid | None (already ellipsoidal) |

## Verification Formula

$$h_{\text{WGS84}}(\phi, \lambda) = H_{\text{orthometric}}(\phi, \lambda) + N_{\text{geoid}}(\phi, \lambda)$$
