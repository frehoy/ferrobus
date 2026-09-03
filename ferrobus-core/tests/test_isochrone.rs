//! Properties of the reachable grid, independent of how it is stored.

use ferrobus_core::algo::{IsochroneIndex, reachable_cells};
use ferrobus_core::{TransitModel, TransitModelConfig, TransitPoint, create_transit_model};
use geo::{Point, Polygon, coord};
use std::collections::HashSet;
use std::path::PathBuf;

fn test_config() -> TransitModelConfig {
    let test_data_dir = PathBuf::from("..").join("tests").join("test-data");
    TransitModelConfig {
        osm_path: test_data_dir.join("roads_zhelez.pbf"),
        gtfs_dirs: vec![test_data_dir.join("zhelez")],
        date: None,
        max_transfer_time: 1200,
    }
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

fn transit_point(model: &TransitModel, lat: f64, lon: f64) -> TransitPoint {
    TransitPoint::new(Point::new(lon, lat), model, 600, 10).expect("Failed to create transit point")
}

/// Two things `calculate_isochrone` cannot be asked about, because dissolving
/// the cells into a polygon throws away which cells they were.
#[test]
fn reached_cells_come_from_the_grid_and_grow_with_the_cutoff() {
    let model = create_transit_model(&test_config()).expect("Failed to create test model");
    let index =
        IsochroneIndex::new(&model, &test_area(), 8, 1200).expect("Index should be created");
    let origin = transit_point(&model, 56.256657, 93.533561);

    let grid: HashSet<_> = index.grid.iter().copied().collect();
    let reached = |cutoff| {
        reachable_cells(&model, &origin, 43200, 2, cutoff, &index)
            .expect("Reachability should be computable")
            .into_iter()
            .collect::<HashSet<_>>()
    };

    let near = reached(900);
    let far = reached(2700);

    assert!(
        !near.is_empty(),
        "a 15 minute cutoff should reach something"
    );

    // Every cell answered is one the index actually holds. A cell from anywhere
    // else would mean the routing results and the grid had drifted apart.
    assert!(near.is_subset(&grid), "reached a cell outside the grid");
    assert!(far.is_subset(&grid), "reached a cell outside the grid");

    // More time cannot reach less ground, and must reach more of it -- without
    // this the assertions above would pass just as well if the cutoff were
    // ignored and every cell returned every time.
    assert!(
        near.is_subset(&far),
        "a 45 minute cutoff dropped cells a 15 minute one reached"
    );
    assert!(
        near.len() < far.len(),
        "the cutoff made no difference: {} cells either way",
        near.len()
    );
    assert!(
        far.len() < grid.len(),
        "a 45 minute cutoff reached the whole grid, so nothing here is constrained"
    );
}
