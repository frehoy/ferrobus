# Actual OSM before/after examples

These maps use the recorded Sweden benchmark pairs and actual ways from `data/sweden/osm_data.osm.pbf`. They are plotted from data, not generated illustrations. The background includes all extracted roads, railways, platforms and building outlines; it is not a walkability classification.

| Pair | Stop name in GTFS | Before | After | PNG |
|---|---|---:|---:|---|
| 58 | Kungstorp | 0 s | 38 s | [Map](01-kungstorp-false-zero.png) |
| 344 | Vallsta by | 654 s | 101 s | [Map](02-vallsta-shorter-route.png) |
| 186 | Årstaberg | 0 s | 371 s | [Map](03-arstaberg-longer-route.png) |

Each map has a matching SVG. Open `index.html` for the gallery. Both panels use identical extents, north-up orientation, and a local metric projection. The Vallsta map includes a separate endpoint-detail inset.

## What is shown

A is the original query coordinate. B is the transit-stop coordinate used as the query destination. Diamonds show the old assigned nodes or the new edge projections. Solid colored lines show the counted network route. Dashed links connect original queries to their attachments: the old direct-walking lookup omitted these costs, while the new one includes them.

- **Kungstorp:** the old A/B queries share one junction. Both new projections are on Kämpingevägen, [OSM way 431574592](https://www.openstreetmap.org/way/431574592). The new result includes 3 seconds of off-street endpoint access.
- **Vallsta by:** the old network path traverses five edges, matched to original OSM ways 482666322, 56687376, 31791873 (two edges), and 707455550. The new result uses partial edges and includes 15 seconds of endpoint access. A projects to gravel road [707455550](https://www.openstreetmap.org/way/707455550); B projects to road 83, [4495317](https://www.openstreetmap.org/way/4495317).
- **Årstaberg:** the old A/B queries again share one junction. New A projects to [1217029049](https://www.openstreetmap.org/way/1217029049), tagged `highway=path`, `informal=yes`, `surface=dirt`. New B projects to [243561938](https://www.openstreetmap.org/way/243561938), tagged `railway=platform`, `public_transport=platform`, `area=yes`. The resulting 41-edge network path loops around the station area and includes 12 seconds of endpoint access. **This is evidence of the algorithm's chosen route, not proof that the attachments or detour are correct.** Levels, platform access and the real pedestrian connection still need investigation.

## Provenance and checks

- Baseline model: `/tmp/issue9-sweden-before.ferrobus`, built from master `68f4b2f`.
- New model: `/tmp/issue9-sweden-compact.ferrobus`, from the edge-snapping draft. OSM and GTFS inputs are identical; hashes are recorded in `docs/benchmarks/issue9-inputs.sha256`.
- `queries.json` records the original benchmark coordinates.
- `routes-before.json` and `routes-after.json` contain traced routes and snap locations. Each tracer asserts that its result exactly equals `TransitPoint::walking_time_to` from the corresponding build. All six assertions passed, reproducing 0/38, 654/101 and 0/371 seconds.
- The baseline discarded edge polylines. `routes-before-osm.json` restores them by matching source/target OSM node IDs to raw ways and checking polyline travel length against each original edge weight (all differences under one second, consistent with integer truncation). It does not draw straight chords between junctions.
- `osm-ways.json` contains the raw way IDs, tags and coordinates used by the maps, filtered from the extraction within 2.5 km of the three queries. Coordinate gaps are not bridged when plotting.
- `tracing/` preserves the diagnostic Rust examples. They ran in temporary checkouts, so the production repository sources were not changed for tracing. The after-checkout exposed `StreetSnap` and `nearest_edge` for diagnostics only.
- `plot_maps.py` creates the PNG/SVG artifacts using Matplotlib. Re-run in an environment with matplotlib installed: `MPLCONFIGDIR=/tmp/ferrobus-map-mpl python plot_maps.py`.

Map data © OpenStreetMap contributors. Way links refer to the live OSM database; drawings and quoted tags reflect the local extract, which may differ from live edits.
