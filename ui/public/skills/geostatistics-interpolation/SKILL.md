---
name: geostatistics-interpolation
description: "Interpolate scattered stations, wells, soil samples or soundings into surfaces with uncertainty: variograms, ordinary/universal/regression kriging, IDW, kriging variance, LOOCV and spatial block CV. Trigger on interpolate, kriging, variogram, IDW; not for point-density heatmaps."
license: MIT
metadata:
  author: Muhammed Enes Duran
---

# Geostatistics & Interpolation

Purpose: interpolation that reports what it doesn't know. The difference
between a professional product and a pretty raster is the uncertainty
surface and an honest cross-validation — both are non-optional here.

## Method selection

| Situation | Method |
|---|---|
| Dense, smooth phenomenon, quick look | IDW (report power parameter; test 1-3) |
| Physical phenomenon with spatial structure, need uncertainty | **Ordinary kriging** (default professional choice) |
| Clear trend (elevation gradient, coastal effect) | Universal kriging or regression kriging on covariates |
| Strong covariates available (DEM, land cover, distances) | Regression kriging / random-forest residual kriging |
| Categorical target | Indicator kriging |
| Honeycomb-free tessellation, no extrapolation wanted | Natural neighbor |

IDW is a reasonable baseline but has no error model and bullseyes around
extremes; say so when delivering IDW-only products.

## Exploratory phase (before any interpolation)

- Map the points with values; look for duplicates at identical coordinates
  (average or offset them — kriging matrices go singular otherwise).
- Histogram + skew: strongly skewed variables (rainfall, concentrations)
  usually want a log/normal-score transform; back-transform predictions
  properly (bias correction for lognormal kriging).
- Trend check: regress value on x, y, and candidate covariates; visible
  trend → universal/regression kriging path.
- Declustering if sampling is preferential (dense where values are high).

## Variogram discipline

The variogram is a MODELING decision, not an auto-fit output:

Install GSTools and PyKrige in your Python/conda environment (alongside matplotlib, NumPy, and SciPy). Run scripts that import GSTools in the environment where it is installed; if your GIS I/O stack (`pandas`, `scikit-learn`, `rasterio`, `geopandas`, `pyproj`) lives in another Python environment, pass arrays and fitted parameters between the two as files.

```python
import gstools as gs

bin_center, gamma = gs.vario_estimate((x, y), values, max_dist=dmax)  # dmax ≈ half extent
model = gs.Exponential(dim=2)
model.fit_variogram(bin_center, gamma, nugget=True)
print(model)   # GSTools reports nugget, var (partial sill), len_scale — plus the fitted plot
```

- Max lag ≈ half the domain diameter; ≥30 pairs per bin.
- Check anisotropy with directional variograms (0/45/90/135°); geological
  and meteorological fields are often anisotropic — fit an anisotropic
  model rather than ignoring it.
- Interpret and report the parameters in words: nugget (measurement error +
  micro-scale variance), range (correlation distance), sill. A nugget near
  the sill means the data barely support interpolation — say that honestly.
- Never interpolate meaningfully beyond the variogram range from the
  nearest sample; mask or flag those cells.

## Kriging execution

```python
from pykrige.ok import OrdinaryKriging

# Pass the fitted GSTools model itself, so PyKrige uses exactly that variogram
ok = OrdinaryKriging(x, y, values, variogram_model=model,
                     coordinates_type="euclidean")
z, ss = ok.execute("grid", gridx, gridy)   # ss = kriging VARIANCE — keep it!
```

Do not copy the GSTools numbers into PyKrige's `variogram_parameters` as they
are: PyKrige's `range` is the practical range (3 × `len_scale` for the
exponential model) and its `sill` is the total sill (`var + nugget`), while
GSTools' `var` is the partial sill. If you need the dict form (exponential):

```python
params = {"sill": model.var + model.nugget, "range": 3 * model.len_scale, "nugget": model.nugget}
ok = OrdinaryKriging(x, y, values, variogram_model="exponential",
                     variogram_parameters=params, coordinates_type="euclidean")
```

Both forms give identical predictions and variances (tested with GSTools 1.7.0, PyKrige 1.7.3).
Other models use other range factors; prefer passing the model object.

- Work in a projected CRS (kriging distances in degrees are wrong except
  with explicitly geographic-aware models).
- Deliver TWO rasters: prediction AND kriging standard deviation
  (`np.sqrt(ss)`). The uncertainty map drives where to sample next and
  where not to trust the map.
- Search neighborhood: 16-32 nearest points typical; document it.

## Validation — leave-one-out and beyond

- **LOOCV** for small n: report ME (bias ≈ 0?), RMSE, and standardized
  RMSE (should be ≈ 1 if kriging variances are honest).
- For clustered samples, LOOCV flatters; also run spatial block CV
  (canonical recipe: `references/spatial-cv-protocol.md`) and report both.
- Scatter plot observed vs predicted with 1:1 line; map the CV residuals —
  spatially clustered residuals mean missing trend/covariate.
- Compare against the dumb baseline (global mean, IDW): kriging must earn
  its complexity.

## Reporting template

```
## Surface: <variable>
- n = <>, extent, CRS, transform applied: <log/none>
- Variogram: <model>, nugget / total sill (var + nugget) / practical range (3 × len_scale for exponential) = <values>, anisotropy: <...>
- Method: <OK/UK/RK + covariates>
- LOOCV: ME <>, RMSE <>, RMSE_std <>; block-CV RMSE <>
- Uncertainty: kriging SD raster delivered; area beyond reliable range masked
```

## Pitfalls checklist

- Auto-fitted variogram accepted without looking at the plot.
- Kriging in EPSG:4326 over large extents.
- Prediction raster delivered without its variance/SD companion.
- Log-kriging back-transformed by naive `exp()` (bias!).
- Extrapolation far beyond the data hull presented at equal confidence.
- Duplicated station coordinates crashing or silently distorting the fit.

## Execution contract

- **Workflow:** inspect sampling and transform needs; model spatial dependence; fit candidate surfaces; validate against simple baselines; quantify uncertainty; mask unsupported extrapolation.
- **Decision rules:** choose IDW or trend methods for transparent baselines, kriging when a defensible variogram exists, and regression kriging when covariates improve blocked validation.
- **Verification protocol:** examine variogram diagnostics, LOOCV and spatial block-CV residuals, baseline uplift, uncertainty calibration, and behavior beyond the sample hull.
- **Failure modes:** withhold a confident surface when sampling is too sparse, dependence is absent, duplicates dominate, distance units are invalid, or uncertainty is uncalibrated.
- **Deliverables:** prediction surface, uncertainty surface, variogram and parameters, validation table, sampling-support mask, and method limitations.
- **Source freshness:** consult [the authoritative source registry](references/authoritative-sources.md) before relying on package APIs or defaults and record the checked date.
