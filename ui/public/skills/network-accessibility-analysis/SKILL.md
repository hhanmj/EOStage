---
name: network-accessibility-analysis
description: "Network travel-cost access by walking, driving, cycling or transit: OSMnx graphs, isochrones, OD matrices, closest facility, 2SFCA, location-allocation, GTFS, equity; flags Euclidean buffers posing as access. Suitability without network costs goes to mcda-suitability-analysis."
license: MIT
---

# Network & Accessibility Analysis

Purpose: replace as-the-crow-flies guesswork with network-true travel
costs, at the right scale and with honest assumptions about speeds and
modes. First decision on every task: Euclidean distance is only acceptable
as a declared approximation — flag it whenever you see it standing in for
access.

## Tool selection by scale

| Scale | Tool |
|---|---|
| Neighborhood-city, research flexibility | **OSMnx + NetworkX** |
| City-region, many-to-many OD (>10⁴×10⁴) | **r5py** (multimodal + transit w/ GTFS) or **pandana** (contraction-hierarchy speed) |
| Production routing service | Valhalla / OSRM (OpenRouteService needs an API key) |

pandana and r5py are not preinstalled: on first use run
`python3 -m pip install --user pandana==0.8` or `python3 -m pip install --user r5py==1.1.7`.

NetworkX chokes on metro-scale many-to-many — don't loop `shortest_path`
over thousands of origins; switch tools instead.

## Graph construction (OSMnx)

```python
import networkx as nx
import osmnx as ox

mode = "drive"                     # or "walk" / "bike"
G = ox.graph_from_point((lat, lon), dist=2000, network_type=mode)
# or ox.graph_from_bbox((west, south, east, north), network_type=mode)
# dist must exceed cutoff × top speed, or isochrones are clipped at the graph edge
if mode == "drive":
    G = ox.add_edge_speeds(G)      # car speeds from maxspeed and road class
else:                              # add_edge_speeds would give car speeds here too
    nx.set_edge_attributes(G, {"walk": 4.8, "bike": 15.0}[mode], "speed_kph")  # assumed; report them
G = ox.add_edge_travel_times(G)    # edge attr: travel_time (s)
G = ox.project_graph(G)            # metric CRS before any distance work
```

- **Prefer coordinates or bbox**: `graph_from_place` and `ox.geocode` call Nominatim,
  which may be rate-limited or unreachable in restricted networks. Take coordinates or a bounding box directly when possible. Call OSMnx directly with its default
  settings: it already waits and retries when Overpass (the OSM download)
  answers 429 or 504, and a 406 from `curl` or bare `requests` is Overpass
  rejecting their default User-Agent, not an outage. If downloads keep
  failing, shrink the area or ask the user for an `.osm.pbf` extract: clip
  and convert with
  `osmium extract -b <west>,<south>,<east>,<north> in.osm.pbf -o area.osm`,
  then `ox.graph_from_xml("area.osm")` (add `bidirectional=True` for walking,
  or one-way streets stay one-way on foot). That loads every way in the file,
  so keep only the mode's roads first (`osmium tags-filter`).
- **network_type matters**: pedestrian analysis on a `drive` graph misses
  paths, stairs, plazas; driving on `all` uses footpaths. Match mode.
- Imputed speeds are averages by road class — a systematic bias, not
  noise. State it; calibrate against known trips when stakes are high.
- Keep the strongly connected component for routing
  (`ox.truncate.largest_component(G, strongly=True)`); orphan islands
  cause spurious infinities.
- **Snapping**: origins/destinations map to nearest nodes/edges
  (`ox.distance.nearest_nodes`). Report the snap-distance distribution;
  a facility snapped 2 km away (riverside, gated area) silently corrupts
  results.

## Core products

- **Isochrones / service areas**: ego-graph by travel_time cutoff → concave
  hull (`shapely.concave_hull`; the `alphashape` package is not installed) or
  buffered edge union around reached edges. Node-based convex
  hulls overstate coverage across rivers/highways — prefer edge-based
  polygons. Always label the assumptions: mode, speed model, cutoff, and
  for areas the polygon method (buffer width; holes kept or filled).
- **OD matrix**: many-to-many travel costs; the substrate for
  accessibility and location-allocation. For big matrices use
  pandana/r5py; store as Parquet with origin/destination IDs.
- **Closest facility**: k-nearest by network cost (not Euclidean); report
  both the assigned facility and the cost.
- **Centrality**: betweenness on travel_time (sampled `k` for big
  graphs — exact is O(nm)); edge betweenness ≈ through-traffic potential.
  Interpret as network structure, not observed traffic.

## Accessibility metrics — pick deliberately

| Metric | Question it answers | Weakness |
|---|---|---|
| Cumulative opportunities (# jobs/POIs within T min) | Simple, communicable | Cliff at T; all-or-nothing |
| Gravity-based (distance-decayed sum) | Smooth access | Decay parameter must be justified |
| **2SFCA / E2SFCA** | Supply-demand ratio access (health care standard) | Catchment size choice drives results |
| Closest-facility time | Worst-case need | Ignores capacity/congestion |

For equity analyses, join metrics to population/demographic polygons
(area-weighted or dasymetric interpolation) and report
distributions per group, not just city means. Route statistical testing of
disparities to `spatial-statistics`.

## Location-allocation

Optimal siting (p-median, max-coverage) on the OD matrix: formulate with
PuLP/OR-Tools; inputs are the OD matrix + demand weights + candidate
sites. State the objective explicitly — minimize mean travel time
(p-median) vs maximize covered demand within T (max-coverage) give
different answers, and stakeholders rarely know which they asked for.
Feed results back to `mcda-suitability-analysis` when siting mixes network
access with other criteria.

## Transit (GTFS)

Use r5py with OSM + GTFS feeds; results are departure-time sensitive —
compute over a time window (e.g., 07:00-09:00 percentiles), never a single
departure. Validate the feed (calendar coverage on your analysis date!) —
an expired GTFS calendar yields walking-only times that look plausible.

In restricted networks where github.com is only intermittently reachable,
the R5 jar r5py downloads on first run may fail to arrive: then
ask the user for the jar (`r5-v7.5.1-r5py-all.jar`
from the r5py/r5 releases) and pass its path as `--r5-classpath <jar>` on
the command line of the script that imports r5py, or state that OSMnx alone
does not model transit. GTFS feeds must come from the user.

## Verification protocol

1. Spot-check 3 routes against an external router — within ~20% or
   explain why. Public endpoints: Valhalla `valhalla1.openstreetmap.de`
   with `costing` set to the mode (`pedestrian`, `bicycle`, `auto`); OSRM
   `router.project-osrm.org` routes cars only, whatever profile the URL
   names, so check walking and cycling against
   `routing.openstreetmap.de/routed-foot/` or `/routed-bike/`.
2. Map unreachable/infinite-cost pairs — usually snapping or connectivity
   artifacts, not real inaccessibility.
3. Isochrone eyeball: does it respect rivers, highways, one-ways? Plot the
   polygon over the graph's own edges with the reached ones highlighted:
   barriers show as gaps in the network, so this check needs no basemap.

## Pitfalls checklist

- Euclidean buffers presented as "service areas".
- Wrong network_type for the mode.
- Convex-hull isochrones bridging barriers.
- Snap distances unchecked.
- One departure time for transit accessibility.
- Betweenness sold as traffic volume.
- OD matrix in degrees-CRS travel "distances".

## Execution contract

- **Workflow:** define mode, time, impedance, origins, destinations, and equity question; build and validate the network; snap inputs; compute routes or matrices; summarize access; verify.
- **Decision rules:** use network costs for constrained travel, movement analytics for observed tracks, and MCDA only when accessibility becomes one criterion in a broader preference model.
- **Verification protocol:** audit connectivity and snapping, spot-check routes, map unreachable pairs, test departure-time or impedance sensitivity, and reconcile OD dimensions and units.
- **Failure modes:** withhold access claims for disconnected graphs, wrong mode or turn rules, expired GTFS service, excessive snapping, Euclidean substitution, or unstable departure-time results.
- **Deliverables:** network provenance, assumptions and cost function, routes or OD matrix, isochrones or access metrics, unreachable-case report, validation evidence, and equity caveats.
- **Source freshness:** consult [the authoritative source registry](references/authoritative-sources.md) before using network, GTFS, or routing APIs and archive source dates.
