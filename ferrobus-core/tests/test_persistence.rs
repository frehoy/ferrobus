use ferrobus_core::algo::IsochroneIndex;
use ferrobus_core::persist::{
    load_isochrone_index, load_or_create_transit_model, load_transit_model, save_isochrone_index,
    save_transit_model,
};
use ferrobus_core::{
    Error, TransitModel, TransitModelConfig, TransitPoint, algo::calculate_isochrone,
    create_transit_model, routing::multimodal_routing::multimodal_routing,
};
use geo::{MultiPolygon, Point, Polygon, coord};
use std::path::PathBuf;
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

fn get_test_data_dir() -> PathBuf {
    PathBuf::from("..").join("tests").join("test-data")
}

fn test_config() -> TransitModelConfig {
    let test_data_dir = get_test_data_dir();
    TransitModelConfig {
        osm_path: test_data_dir.join("roads_zhelez.pbf"),
        gtfs_dirs: vec![test_data_dir.join("zhelez")],
        date: None,
        max_transfer_time: 1200,
    }
}

fn temp_path(name: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should be valid")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ferrobus_persist_{name}_{}_{ts}.ferrobus",
        process::id()
    ))
}

fn transit_point(model: &TransitModel, lat: f64, lon: f64) -> TransitPoint {
    TransitPoint::new(Point::new(lon, lat), model, 600, 10).expect("Failed to create transit point")
}

/// The area covered by the bundled test feed.
fn test_area() -> Polygon {
    Polygon::new(
        vec![
            coord! { x: 93.39795011002934, y: 56.18357044999381 },
            coord! { x: 93.57274857628481, y: 56.18357044999381 },
            coord! { x: 93.57274857628481, y: 56.30437667924404 },
            coord! { x: 93.39795011002934, y: 56.30437667924404 },
            coord! { x: 93.39795011002934, y: 56.18357044999381 },
        ]
        .into(),
        vec![],
    )
}

#[test]
fn test_transit_model_round_trip() {
    let model = create_transit_model(&test_config()).expect("Failed to create test model");
    let path = temp_path("model_round_trip");

    save_transit_model(&model, &path).expect("Model should be saved");
    let loaded = load_transit_model(&path).expect("Model should be loaded");
    std::fs::remove_file(&path).ok();

    assert_eq!(loaded.stop_count(), model.stop_count());
    assert_eq!(loaded.route_count(), model.route_count());
    assert_eq!(
        loaded.transit_data.stop_times.len(),
        model.transit_data.stop_times.len()
    );
    assert_eq!(
        loaded.transit_data.transfers.len(),
        model.transit_data.transfers.len()
    );
    assert_eq!(
        loaded.street_graph.graph.node_count(),
        model.street_graph.graph.node_count()
    );
    assert_eq!(
        loaded.street_graph.graph.edge_count(),
        model.street_graph.graph.edge_count()
    );
    assert_eq!(loaded.meta.max_transfer_time, model.meta.max_transfer_time);
}

#[test]
fn test_loaded_model_routes_identically() {
    let model = create_transit_model(&test_config()).expect("Failed to create test model");
    let path = temp_path("routing_parity");

    save_transit_model(&model, &path).expect("Model should be saved");
    let loaded = load_transit_model(&path).expect("Model should be loaded");
    std::fs::remove_file(&path).ok();

    // The R-tree is persisted too, so snapping must land on the same node.
    let start_before = transit_point(&model, 56.256657, 93.533561);
    let end_before = transit_point(&model, 56.242574, 93.499159);
    let start_after = transit_point(&loaded, 56.256657, 93.533561);
    let end_after = transit_point(&loaded, 56.242574, 93.499159);

    assert_eq!(start_after.node_id, start_before.node_id);
    assert_eq!(end_after.node_id, end_before.node_id);

    let before = multimodal_routing(&model, &start_before, &end_before, 43200, 2)
        .expect("Routing failed")
        .expect("Must return a route");
    let after = multimodal_routing(&loaded, &start_after, &end_after, 43200, 2)
        .expect("Routing failed")
        .expect("Must return a route");

    assert_eq!(after.travel_time, before.travel_time);
    assert_eq!(after.transfers, before.transfers);
    assert_eq!(after.walking_time, before.walking_time);
}

#[test]
fn test_isochrone_index_round_trip() {
    let model = create_transit_model(&test_config()).expect("Failed to create test model");
    let index =
        IsochroneIndex::new(&model, &test_area(), 8, 1200).expect("Index should be created");
    let path = temp_path("index_round_trip");

    save_isochrone_index(&index, &path).expect("Index should be saved");
    let loaded = load_isochrone_index(&path).expect("Index should be loaded");
    std::fs::remove_file(&path).ok();

    assert_eq!(loaded.len(), index.len());
    assert_eq!(loaded.resolution(), index.resolution());
    assert_eq!(loaded.grid, index.grid);

    // An isochrone computed from the reloaded index must cover the same ground.
    let origin = transit_point(&model, 56.256657, 93.533561);
    let before = calculate_isochrone(&model, &origin, 43200, 2, 1800, &index)
        .expect("Isochrone calculation failed");
    let after = calculate_isochrone(&model, &origin, 43200, 2, 1800, &loaded)
        .expect("Isochrone calculation failed");

    // The dissolve step does not pin which vertex a ring starts at, so compare
    // the vertex sets rather than the geometries verbatim.
    assert_eq!(after.0.len(), before.0.len());
    assert_eq!(vertex_set(&after), vertex_set(&before));
}

/// All polygon vertices of a multipolygon, in a rotation-independent order.
///
/// The closing vertex of each ring is dropped, since which vertex a ring is
/// closed on shifts with its rotation.
fn vertex_set(geometry: &MultiPolygon) -> Vec<(String, String)> {
    let mut coords: Vec<(String, String)> = geometry
        .0
        .iter()
        .flat_map(|polygon| {
            let ring = &polygon.exterior().0;
            &ring[..ring.len().saturating_sub(1)]
        })
        .map(|coord| (format!("{:.9}", coord.x), format!("{:.9}", coord.y)))
        .collect();
    coords.sort();
    coords
}

#[test]
fn test_load_or_create_builds_then_reuses_cache() {
    let path = temp_path("cache");
    let config = test_config();

    assert!(!path.exists());
    let built = load_or_create_transit_model(&config, &path).expect("Model should be built");
    assert!(path.exists(), "cache file should have been written");

    let cached = load_or_create_transit_model(&config, &path).expect("Model should be loaded");
    std::fs::remove_file(&path).ok();

    assert_eq!(cached.stop_count(), built.stop_count());
    assert_eq!(cached.route_count(), built.route_count());
}

#[test]
fn test_load_rejects_foreign_file() {
    let path = temp_path("foreign");
    std::fs::write(&path, b"this is definitely not a transit model")
        .expect("temp file should be written");

    let result = load_transit_model(&path);
    std::fs::remove_file(&path).ok();

    assert!(matches!(result, Err(Error::IncompatibleFormat(_))));
}

#[test]
fn test_load_rejects_truncated_file() {
    let model = create_transit_model(&test_config()).expect("Failed to create test model");
    let path = temp_path("truncated");
    save_transit_model(&model, &path).expect("Model should be saved");

    let bytes = std::fs::read(&path).expect("file should be readable");
    std::fs::write(&path, &bytes[..bytes.len() / 2]).expect("file should be truncated");

    let result = load_transit_model(&path);
    std::fs::remove_file(&path).ok();

    assert!(result.is_err(), "a truncated file must not load");
}

#[test]
fn test_load_missing_file_is_io_error() {
    let result = load_transit_model(temp_path("missing"));
    assert!(matches!(result, Err(Error::IoError(_))));
}

/// A persisted index is only worth caching if rebuilding it gives the same file.
#[test]
fn test_isochrone_index_build_is_reproducible() {
    let model = create_transit_model(&test_config()).expect("Failed to create test model");

    let first =
        IsochroneIndex::new(&model, &test_area(), 9, 1200).expect("Index should be created");
    let second =
        IsochroneIndex::new(&model, &test_area(), 9, 1200).expect("Index should be created");

    assert_eq!(first.grid, second.grid);

    // Compare through the serialised form, since the per-cell data is private.
    let first_path = temp_path("reproducible_first");
    let second_path = temp_path("reproducible_second");
    save_isochrone_index(&first, &first_path).expect("Index should be saved");
    save_isochrone_index(&second, &second_path).expect("Index should be saved");

    let first_bytes = std::fs::read(&first_path).expect("Index should be readable");
    let second_bytes = std::fs::read(&second_path).expect("Index should be readable");
    std::fs::remove_file(&first_path).ok();
    std::fs::remove_file(&second_path).ok();

    assert_eq!(first_bytes, second_bytes);
}
