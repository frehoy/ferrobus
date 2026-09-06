//! Memory probe for the build-vs-load and H3-resolution questions.
//!
//! Each subcommand is meant to be run as its own process, so the peak RSS it
//! reports is the peak of that stage alone rather than of a whole pipeline.
//!
//! ```text
//! cargo run --release --example memprobe -- build-model  <dataset> <model.ferrobus>
//! cargo run --release --example memprobe -- load-model   <model.ferrobus>
//! cargo run --release --example memprobe -- build-index  <model.ferrobus> <res> <index.ferrobus>
//! cargo run --release --example memprobe -- load-index   <model.ferrobus> <index.ferrobus>
//! cargo run --release --example memprobe -- isochrone    <model.ferrobus> <index.ferrobus> <lng> <lat>
//! cargo run --release --example memprobe -- route        <model.ferrobus> <from_lng> <from_lat> <to_lng> <to_lat>
//! cargo run --release --example memprobe -- walk-path    <model.ferrobus> <from_lng> <from_lat> <to_lng> <to_lat>
//! ```
//!
//! Environment:
//!
//! | Variable | Meaning |
//! |---|---|
//! | `FERROBUS_DATE` | `YYYY-MM-DD` to filter to one service day; unset keeps the whole feed |
//! | `FERROBUS_OSM` | override the `.pbf` path, for a directory laid out differently |
//! | `FERROBUS_GTFS` | override the feed directory, likewise |
//! | `FERROBUS_MAX_TRANSFER_TIME` | seconds; default 1200 |
//! | `FERROBUS_RSS_SAMPLE_SECS` | RSS trace period, `0` to disable; default 1 |
//! | `RUST_LOG` | log level for ferrobus's own phase markers; default `info` |

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::too_many_lines
)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use ferrobus_core::WALKING_SPEED;
use ferrobus_core::persist;
use ferrobus_core::prelude::*;
use geo::{Coord, LineString, Point, Polygon};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use std::collections::HashMap;
use wkt::ToWkt;

/// Peak resident set size of this process, in bytes.
fn peak_rss() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) } != 0 {
        return 0;
    }
    let max_rss = usage.ru_maxrss as u64;
    // Darwin reports bytes, Linux reports kilobytes.
    if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss * 1024
    }
}

/// Resident set size right now, in bytes; Linux avoids forking `ps` per sample.
#[cfg(target_os = "linux")]
fn current_rss() -> u64 {
    // statm field 2 is the resident set, in pages.
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let pages: u64 = statm
        .split_whitespace()
        .nth(1)
        .and_then(|field| field.parse().ok())
        .unwrap_or(0);
    // Asked for, not assumed: aarch64 pages are 4 KiB on Debian, 64 KiB on RHEL.
    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    pages * u64::try_from(page_size).unwrap_or(4096)
}

/// Resident set size right now, in bytes.
#[cfg(not(target_os = "linux"))]
fn current_rss() -> u64 {
    let pid = std::process::id();
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output();
    match out {
        Ok(out) => {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<u64>()
                .unwrap_or(0)
                * 1024
        }
        Err(_) => 0,
    }
}

/// Prints RSS periodically, so a peak inside a phase is visible, not just at its edges.
fn spawn_rss_sampler(started: Instant) {
    let period = std::env::var("FERROBUS_RSS_SAMPLE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    if period == 0 {
        return;
    }

    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(period));
            println!(
                "[rss]  t={:>7.1}s  rss={:>7.2} GiB  peak={:>7.2} GiB",
                started.elapsed().as_secs_f64(),
                gib(current_rss()),
                gib(peak_rss())
            );
        }
    });
}

fn gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// Prints a checkpoint line: where we are, RSS now, peak so far, elapsed.
fn mark(label: &str, started: Instant) {
    println!(
        "[mark] {label:<28} rss={:>7.2} GiB  peak={:>7.2} GiB  t={:>7.1}s",
        gib(current_rss()),
        gib(peak_rss()),
        started.elapsed().as_secs_f64()
    );
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |m| m.len())
}

fn dataset_config(dataset: &str) -> TransitModelConfig {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("data")
        .join(dataset);

    // `none` must mean no filtering; a combinator chain would let the default back in.
    let date = match std::env::var("FERROBUS_DATE") {
        Ok(value) if value.eq_ignore_ascii_case("none") => None,
        Ok(value) => Some(
            chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                .expect("FERROBUS_DATE should be YYYY-MM-DD or 'none'"),
        ),
        // No filter unless asked: a fixed default date silently falls outside a
        // feed's validity window and turns the probe into an empty model.
        Err(_) => None,
    };

    // Overridable, to point the probe at a directory laid out some other way.
    let osm_path = std::env::var_os("FERROBUS_OSM")
        .map_or_else(|| root.join("osm_data.osm.pbf"), PathBuf::from);
    let gtfs_dirs = std::env::var_os("FERROBUS_GTFS")
        .map_or_else(|| vec![root.join("gtfs")], |dir| vec![PathBuf::from(dir)]);

    let max_transfer_time: Time = std::env::var("FERROBUS_MAX_TRANSFER_TIME")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1200);

    println!(
        "dataset={dataset} date={date:?} max_transfer_time={max_transfer_time} osm={} gtfs={}",
        osm_path.display(),
        gtfs_dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(","),
    );

    TransitModelConfig {
        gtfs_dirs,
        osm_path,
        max_transfer_time,
        date,
    }
}

/// Bounding box of the routable street network; stop extents include junk coordinates.
fn area_polygon(model: &TransitModel) -> Polygon {
    let (mut min_x, mut min_y) = (f64::MAX, f64::MAX);
    let (mut max_x, mut max_y) = (f64::MIN, f64::MIN);
    for node in model.street_graph().graph.node_weights() {
        let (x, y) = node.geometry.x_y();
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    println!("area bbox: [{min_x:.4}, {min_y:.4}, {max_x:.4}, {max_y:.4}]");
    Polygon::new(
        LineString::from(vec![
            Coord { x: min_x, y: min_y },
            Coord { x: max_x, y: min_y },
            Coord { x: max_x, y: max_y },
            Coord { x: min_x, y: max_y },
            Coord { x: min_x, y: min_y },
        ]),
        vec![],
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    env_logger_init(started);
    spawn_rss_sampler(started);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map_or("help", String::as_str);

    match command {
        "build-model" => {
            let dataset = &args[1];
            let out = PathBuf::from(&args[2]);
            let config = dataset_config(dataset);
            mark("start", started);
            let model = create_transit_model(&config)?;
            mark("model built", started);
            println!(
                "stops={} routes={}",
                model.stop_count(),
                model.route_count()
            );
            persist::save_transit_model(&model, &out)?;
            mark("model saved", started);
            println!("model file: {:.2} GiB", gib(file_size(&out)));
        }
        "load-model" => {
            let path = PathBuf::from(&args[1]);
            println!("model file: {:.2} GiB", gib(file_size(&path)));
            mark("start", started);
            let model = persist::load_transit_model(&path)?;
            mark("model loaded", started);
            println!(
                "stops={} routes={}",
                model.stop_count(),
                model.route_count()
            );
        }
        "build-index" => {
            let model_path = PathBuf::from(&args[1]);
            let resolution: u8 = args[2].parse()?;
            let out = PathBuf::from(&args[3]);
            let max_walking_time: Time = args
                .get(4)
                .map(|value| value.parse())
                .transpose()?
                .unwrap_or(1200);

            mark("start", started);
            let model = persist::load_transit_model(&model_path)?;
            mark("model loaded", started);
            let area = area_polygon(&model);
            let index = IsochroneIndex::new(&model, &area, resolution, max_walking_time)?;
            mark(&format!("index built (res {resolution})"), started);
            println!("index cells={}", index.len());
            persist::save_isochrone_index(&index, &out)?;
            mark("index saved", started);
            println!("index file: {:.2} GiB", gib(file_size(&out)));
        }
        "load-index" => {
            let model_path = PathBuf::from(&args[1]);
            let index_path = PathBuf::from(&args[2]);
            println!(
                "model file: {:.2} GiB  index file: {:.2} GiB",
                gib(file_size(&model_path)),
                gib(file_size(&index_path))
            );
            mark("start", started);
            let model = persist::load_transit_model(&model_path)?;
            mark("model loaded", started);
            let index = persist::load_isochrone_index(&index_path)?;
            mark("index loaded", started);
            println!(
                "stops={} cells={} res={}",
                model.stop_count(),
                index.len(),
                index.resolution()
            );
        }
        // Whether two models still answer the same journeys.
        "route" => {
            let model_path = PathBuf::from(&args[1]);
            let coords: Vec<f64> = args[2..6]
                .iter()
                .map(|value| value.parse())
                .collect::<Result<_, _>>()?;
            let departure_time: Time = args.get(6).map_or(Ok(28800), |v| v.parse())?;
            let max_transfers: usize = args.get(7).map_or(Ok(3), |v| v.parse())?;

            mark("start", started);
            let model = persist::load_transit_model(&model_path)?;
            mark("model loaded", started);
            println!(
                "street nodes={} edges={} stops={} routes={}",
                model.street_graph().graph.node_count(),
                model.street_graph().graph.edge_count(),
                model.stop_count(),
                model.route_count()
            );

            // node_id identifies an endpoint; query costs also include the partial edge.
            let snap_report = |label: &str, point: Point<f64>, tp: &TransitPoint| {
                let node = model.street_graph().graph[tp.node_id].geometry;
                let (dx, dy) = (node.x() - point.x(), node.y() - point.y());
                // Rough metres, good enough to compare two snaps.
                let metres = ((dy * 111_320.0).powi(2)
                    + (dx * 111_320.0 * point.y().to_radians().cos()).powi(2))
                .sqrt();
                println!(
                    "  {label}: endpoint node={} at ({:.5}, {:.5}), {metres:.0} m from query",
                    tp.node_id.index(),
                    node.x(),
                    node.y()
                );
                // Which stops this point can board at, and how far the walk is.
                for stop in tp.nearest_stops() {
                    let at = tp.transit_stop_location(&model.transit_data, stop);
                    let walk = TransitPoint::new(at, &model, 1200, 1)
                        .ok()
                        .and_then(|sp| tp.walking_time_to(&sp));
                    println!(
                        "      stop {stop} {:?} at ({:.5}, {:.5}) walk={walk:?}s",
                        tp.transit_stop_name(&model.transit_data, stop)
                            .unwrap_or_default(),
                        at.x(),
                        at.y()
                    );
                }
            };

            let from_pt = Point::new(coords[0], coords[1]);
            let to_pt = Point::new(coords[2], coords[3]);
            let from = TransitPoint::new(from_pt, &model, 1200, 3)?;
            let to = TransitPoint::new(to_pt, &model, 1200, 3)?;
            snap_report("from", from_pt, &from);
            snap_report("to", to_pt, &to);
            println!("direct walking={:?}s", from.walking_time_to(&to));
            match multimodal_routing(&model, &from, &to, departure_time, max_transfers)? {
                Some(result) => println!(
                    "travel={}s walking={}s transit={:?}s transfers={}",
                    result.travel_time, result.walking_time, result.transit_time, result.transfers
                ),
                None => println!("no route found"),
            }
            mark("routed", started);
        }
        // The walking route between two points, so a detour can be seen rather
        // than inferred from its duration.
        "walk-path" => {
            println!("Node-to-node diagnostic; query routing uses edge projections.");
            let model_path = PathBuf::from(&args[1]);
            let coords: Vec<f64> = args[2..6]
                .iter()
                .map(|value| value.parse())
                .collect::<Result<_, _>>()?;

            mark("start", started);
            let model = persist::load_transit_model(&model_path)?;
            mark("model loaded", started);

            let graph = &model.street_graph().graph;
            let snap = |lng: f64, lat: f64| {
                model
                    .street_graph()
                    .rtree
                    .nearest_neighbor(&Point::new(lng, lat))
                    .expect("point should snap to the network")
                    .data
            };
            let (from, to) = (snap(coords[0], coords[1]), snap(coords[2], coords[3]));

            // Plain Dijkstra over the public graph, keeping predecessors so the
            // route itself can be printed.
            let mut dist: HashMap<NodeIndex, u32> = HashMap::new();
            let mut prev: HashMap<NodeIndex, NodeIndex> = HashMap::new();
            let mut heap = std::collections::BinaryHeap::new();
            dist.insert(from, 0);
            heap.push(std::cmp::Reverse((0u32, from.index())));

            while let Some(std::cmp::Reverse((cost, raw))) = heap.pop() {
                let node = NodeIndex::new(raw);
                if node == to {
                    break;
                }
                if cost > *dist.get(&node).unwrap_or(&u32::MAX) {
                    continue;
                }
                for edge in graph.edges(node) {
                    let next = if edge.source() == node {
                        edge.target()
                    } else {
                        edge.source()
                    };
                    let step = cost + edge.weight().weight;
                    if step < *dist.get(&next).unwrap_or(&u32::MAX) {
                        dist.insert(next, step);
                        prev.insert(next, node);
                        heap.push(std::cmp::Reverse((step, next.index())));
                    }
                }
            }

            match dist.get(&to) {
                None => println!("no walking path"),
                Some(&seconds) => {
                    let mut path = vec![to];
                    while let Some(&p) = prev.get(path.last().expect("non-empty")) {
                        path.push(p);
                    }
                    path.reverse();

                    let metres = f64::from(seconds) * WALKING_SPEED;
                    let straight = {
                        let (a, b) = (graph[from].geometry, graph[to].geometry);
                        let (dx, dy) = (b.x() - a.x(), b.y() - a.y());
                        ((dy * 111_320.0).powi(2)
                            + (dx * 111_320.0 * a.y().to_radians().cos()).powi(2))
                        .sqrt()
                    };
                    println!(
                        "walk={seconds}s  {metres:.0} m over {} nodes  straight line {straight:.0} m  detour x{:.1}",
                        path.len(),
                        metres / straight
                    );
                    let wkt: Vec<String> = path
                        .iter()
                        .map(|n| {
                            let p = graph[*n].geometry;
                            format!("{:.6} {:.6}", p.x(), p.y())
                        })
                        .collect();
                    println!("LINESTRING({})", wkt.join(", "));
                }
            }
            mark("routed", started);
        }
        // What is actually mapped around a point; a built model keeps no tags.
        "ways-near" => {
            let pbf = PathBuf::from(&args[1]);
            let lng: f64 = args[2].parse()?;
            let lat: f64 = args[3].parse()?;
            let radius: f64 = args.get(4).map_or(Ok(150.0), |v| v.parse())?;

            let dlat = radius / 111_320.0;
            let dlng = radius / (111_320.0 * lat.to_radians().cos());
            let (min_lng, max_lng) = (lng - dlng, lng + dlng);
            let (min_lat, max_lat) = (lat - dlat, lat + dlat);
            println!("bbox [{min_lng:.5}, {min_lat:.5}, {max_lng:.5}, {max_lat:.5}]");

            // Nodes precede ways in a pbf, so one pass suffices.
            let mut in_box: std::collections::HashSet<i64> = std::collections::HashSet::new();
            let mut ways = 0usize;
            let mut summary: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();

            osmpbf::ElementReader::from_path(&pbf)?.for_each(|element| match element {
                osmpbf::Element::Node(n) => {
                    if n.lon() >= min_lng
                        && n.lon() <= max_lng
                        && n.lat() >= min_lat
                        && n.lat() <= max_lat
                    {
                        in_box.insert(n.id());
                    }
                }
                osmpbf::Element::DenseNode(n) => {
                    if n.lon() >= min_lng
                        && n.lon() <= max_lng
                        && n.lat() >= min_lat
                        && n.lat() <= max_lat
                    {
                        in_box.insert(n.id());
                    }
                }
                osmpbf::Element::Way(w) => {
                    if !w.refs().any(|r| in_box.contains(&r)) {
                        return;
                    }
                    ways += 1;
                    let tags: Vec<String> = w.tags().map(|(k, v)| format!("{k}={v}")).collect();
                    // Group by the tag that decides routability.
                    let kind = [
                        "highway",
                        "railway",
                        "public_transport",
                        "indoor",
                        "building",
                        "area:highway",
                    ]
                    .iter()
                    .find_map(|key| {
                        w.tags()
                            .find(|(k, _)| k == key)
                            .map(|(k, v)| format!("{k}={v}"))
                    })
                    .unwrap_or_else(|| "(other)".to_string());
                    *summary.entry(kind).or_default() += 1;
                    println!("way {} [{}]", w.id(), tags.join(", "));
                }
                osmpbf::Element::Relation(_) => {}
            })?;

            println!("\n{} nodes in box, {ways} ways touching it", in_box.len());
            for (kind, count) in &summary {
                println!("  {count:>4}  {kind}");
            }
            mark("scanned", started);
        }
        "isochrone" => {
            let model_path = PathBuf::from(&args[1]);
            let index_path = PathBuf::from(&args[2]);
            let lng: f64 = args[3].parse()?;
            let lat: f64 = args[4].parse()?;
            let cutoff: Time = args.get(5).map(|v| v.parse()).transpose()?.unwrap_or(1800);

            mark("start", started);
            let model = persist::load_transit_model(&model_path)?;
            let index = persist::load_isochrone_index(&index_path)?;
            mark("model+index loaded", started);

            let point = TransitPoint::new(geo::Point::new(lng, lat), &model, 1200, 3)?;
            let query_start = Instant::now();
            let isochrone = calculate_isochrone(&model, &point, 12 * 3600, 3, cutoff, &index)?;
            mark("isochrone computed", started);
            println!(
                "polygons={} query={:.2}s",
                isochrone.0.len(),
                query_start.elapsed().as_secs_f64()
            );
            if let Ok(path) = std::env::var("FERROBUS_ISOCHRONE_OUT") {
                std::fs::write(&path, isochrone.wkt_string())?;
                println!("wrote {path}");
            }
            if let Ok(path) = std::env::var("FERROBUS_CELLS_OUT") {
                let mut cells = ferrobus_core::algo::reachable_cells(
                    &model,
                    &point,
                    12 * 3600,
                    3,
                    cutoff,
                    &index,
                )?;
                cells.sort_unstable();
                let mut dump = String::new();
                for cell in &cells {
                    use std::fmt::Write;
                    writeln!(dump, "{cell}")?;
                }
                std::fs::write(&path, dump)?;
                println!("wrote {} cells to {path}", cells.len());
            }
        }
        _ => {
            eprintln!(
                "usage: memprobe <build-model|load-model|build-index|load-index|isochrone> ..."
            );
            std::process::exit(2);
        }
    }

    println!(
        "[final] peak={:.2} GiB  total={:.1}s",
        gib(peak_rss()),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

/// Start of the run, so log records and RSS samples share one clock.
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Prints ferrobus's `log` records on the same elapsed clock as the RSS trace.
struct ElapsedLogger;

impl log::Log for ElapsedLogger {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    fn log(&self, record: &log::Record<'_>) {
        let elapsed = START.get().map_or(0.0, |s| s.elapsed().as_secs_f64());
        println!(
            "[log]  t={elapsed:>7.1}s  {:<5} {}",
            record.level(),
            record.args()
        );
    }

    fn flush(&self) {}
}

fn env_logger_init(started: Instant) {
    static LOGGER: ElapsedLogger = ElapsedLogger;

    let _ = START.set(started);
    let level = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(log::LevelFilter::Info);
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}
