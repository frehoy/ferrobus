# Edge snapping: draft evaluation

This prototype implements #9's edge projections. The paired national measurements and traced OSM examples below distinguish corrected undercounting from shorter network routes; they do not establish that every station attachment is correct.

## Implementation

Retain walkable OSM polylines, including closed ways. Index each edge's bounding box in unit-sphere coordinates; search candidate polylines until no remaining box can improve the nearest segment. This avoids longitude/latitude distance bias and storing another copy of every geometry segment in the index. Projection uses unit-sphere chords, an approximation appropriate to short OSM segments, rather than exact ellipsoidal geodesics.

Project stops before transfer calculation. Split each affected edge in deterministic fraction/stop order and add explicit off-street connectors. Cumulative rounded costs conserve the original edge weight across splits. Coincident stops share their attachment, and artificial connectors are excluded from subsequent edge searches.

Query points use two initial Dijkstra costs, one for each endpoint of the projected edge. Direct walking also considers the interval between two projections on the same edge. Walking costs include endpoint connectors and respect the origin's walking budget. One-to-many routing and H3 cells retain the destination attachment.

Saved models and H3 indexes use format **3** and must be rebuilt. An H3 cell now has a size budget of 64 bytes rather than 32; national H3 index memory/build-time measurements remain outstanding.

## Inputs and procedure

Baseline: master at `68f4b2fe8c0fa6e2d7473b07ba46e4cf34f4a9eb`. Both builds use the local Sweden OSM extract and GTFS feed (feed version `2026-08-04`), without service-date filtering, with a 1200-second transfer limit. File hashes are in [issue9-inputs.sha256](issue9-inputs.sha256).

These are single-process measurements on the same macOS machine, not repeated performance trials. `memprobe` reports peak RSS using `getrusage`; its current-RSS readings were unavailable in the sandbox and are not used.

| Metric | master | Draft |
|---|---:|---:|
| Build and save, seconds | 30.4 | 35.0 |
| Peak RSS, GiB | 4.82 | 4.81 |
| Saved model, GiB | 0.32 | 1.14 |
| Sample median query time, microseconds | 37 | 161.5 |

The query timer includes construction of both endpoints and a multimodal query, excluding model loading. Results are indicative; sample cases and returned routes differ between builds.

## Walking sample

`access_probe` chooses 100 evenly spaced stops in feed order and offsets each by approximately 50 metres in four cardinal directions: 400 fixed origin/destination pairs. This is a diagnostic sample, not a representative national accuracy estimate. Endpoints are identical between builds.

| Metric, seconds unless stated | master | Draft |
|---|---:|---:|
| Reachable within 1200s, pairs | 396 | 396 |
| Zero-duration walks, pairs | 168 | 0 |
| Median reachable walking time | 22 | 54 |
| 95th percentile (nearest lower rank) | 189 | 181 |
| Maximum reachable walking time | 759 | 610 |

Among the 396 mutually reachable pairs, 71 become shorter, 3 are unchanged, and 322 become longer. The old direct-walking lookup omitted endpoint connectors and collapsed distinct points at a shared junction, explaining why zero-duration baselines cannot serve as accuracy targets. Longer results still need inspection: neither a reduced maximum nor eliminating zeroes proves the chosen paths are correct.

The largest increase is 0 to 371 seconds between `(18.027569, 59.2997921555875)` and `(18.027569, 59.299343)`. Review the actual street connectivity and chosen projections before treating this as a justified correction.

Raw sample outputs: [before](issue9-access-before.jsonl), [after](issue9-access-after.jsonl). Stop ID lists are omitted from these copies; the probe emits them for investigation.

## Traced OSM examples

See [the map gallery](issue9-maps/index.html) and [tracing details](issue9-maps/README.md). All six trace costs match the production routing output. The original baseline polylines were recovered from raw OSM ways by matching their endpoint node IDs and edge weights.

- [Kungstorp, pair 58](issue9-maps/01-kungstorp-false-zero.png): 0 → 38 seconds. Two points previously collapsed onto one junction; both new projections lie on Kämpingevägen.
- [Vallsta by, pair 344](issue9-maps/02-vallsta-shorter-route.png): 654 → 101 seconds. Partial-edge routing avoids a five-edge junction detour.
- [Årstaberg, pair 186](issue9-maps/03-arstaberg-longer-route.png): 0 → 371 seconds. The old lookup again collapses both queries onto one junction; the new attachments are an informal path and a platform area, with a long network loop between them. The map explains the counted route; attachment correctness is not established.

## Station checks

Coordinates are in [issue9-queries.json](issue9-queries.json), in table order. Departures are at 08:00, with three transfers allowed and three nearby stops retained. The router's existing single-target candidate limit remains in effect.

| Pair | master total / walking seconds | Draft total / walking seconds |
|---|---:|---:|
| Stockholm C → Terminal 5, exact #8 endpoints | no route | no route |
| Stockholm C → another Arlanda point | 1616 / 34 | 1923 / 172 |
| Nearby Malmö graph coordinates from #9 | 207 / 27 (transit); direct walk 595 | 597 / 597 (walking) |
| Malmö point → Lund C point | 1830 / 31 | 2048 / 616 |
| Lund C point → Helsingborg C point | 3333 / 63 | 3800 / 52 |

Only Terminal 5 uses the complete historical query supplied in its issue. The Malmö diagnostic uses the first and third coordinates printed in #9's path, **not** the missing destination of its 48-metre example. These checks are not reproductions of #4's eleven historical city pairs. They establish that this prototype alone does not resolve Terminal 5 or the nearby Malmö detour and that several journeys regress.

## Validation and remaining review

Controlled tests cover same-edge queries beyond either junction's search budget, proportional costs, curved polylines, multiple and coincident stops, off-street connector costs and distance limits, deterministic splitting, closed ways, high-latitude snapping, bounding-box candidate ordering, and preserving separation between nearby streets. The existing model-build reproducibility and persistence round-trip tests exercise the new representation. Rust and Python journey snapshots are updated for the new attachment costs.

Before marking ready:

- Inspect additional station cases and explain journey regressions; recovering the exact historical Malmö query is not required for these comparisons.
- Inspect worst-case sampled walks for wrong-side, barrier, or level attachments. Nearest-edge projection has no new barrier/level awareness.
- Measure H3 index memory/build time and repeated query latency; assess the larger persisted models.
- Use the archived tracing helpers for further path inspection. The existing `memprobe walk-path` remains a node-based diagnostic; `access_probe` reports projected query costs.

## Reproduction

Run each revision's `memprobe` against the same input directory, saving separate model files. Copy `access_probe.rs` into the baseline checkout before building that example; it uses APIs available on master.

```sh
FERROBUS_OSM=/absolute/path/osm_data.osm.pbf \
FERROBUS_GTFS=/absolute/path/gtfs \
FERROBUS_RSS_SAMPLE_SECS=0 \
cargo run --release -p ferrobus_core --example memprobe -- build-model sweden /tmp/model.ferrobus

cargo run --release -p ferrobus_core --example access_probe -- /tmp/model.ferrobus > /tmp/access.jsonl
cargo run --release -p ferrobus_core --example access_probe -- /tmp/model.ferrobus docs/benchmarks/issue9-queries.json
```
