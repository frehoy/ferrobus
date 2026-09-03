//! Calculation of isochrones with naive buffer over reached nodes
//! can be very slow for large areas. This module provides an
//! alternative approach to calculate isochrones using H3 hexagonal
//! grid cells as a index.

use geo::{MultiPolygon, Point, Polygon};
use hashbrown::HashMap;
use log::info;
use petgraph::graph::NodeIndex;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use h3o::{
    CellIndex, LatLng, Resolution,
    geom::{ContainmentMode, SolventBuilder, TilerBuilder},
};

use crate::routing::multimodal_routing::RoutingTarget;
use crate::{Error, RaptorStopId, Time, TransitModel};
use crate::{TransitPoint, multimodal_routing_one_to_many};

/// Egress stops kept per grid cell; three is what the index has always kept.
const GRID_EGRESS_STOPS: usize = 3;

/// How many candidate cells are snapped at a time while building the index.
const SNAP_CHUNK: usize = 1 << 16;

/// One grid cell's connection to the transit network: 32 bytes, heap-free.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct GridPoint {
    /// Street network node the cell centroid snapped to.
    node: u32,
    /// How many entries of `stops` are populated.
    stop_count: u8,
    /// `(stop, walking seconds)` pairs, nearest first.
    stops: [(u32, u32); GRID_EGRESS_STOPS],
}

impl GridPoint {
    /// Snaps a centroid, or `None` if it is out of walking range.
    fn snap(
        centroid: Point<f64>,
        transit_model: &TransitModel,
        max_walking_time: Time,
    ) -> Option<Self> {
        let (node, nearest_stops) = TransitPoint::snap_destination(
            centroid,
            transit_model,
            max_walking_time,
            GRID_EGRESS_STOPS,
        )
        .ok()?;

        let node = u32::try_from(node.index()).ok()?;
        let mut stops = [(0, 0); GRID_EGRESS_STOPS];
        let mut stop_count = 0u8;

        for (slot, &(stop, time)) in stops.iter_mut().zip(&nearest_stops) {
            *slot = (u32::try_from(stop).ok()?, time);
            stop_count += 1;
        }

        Some(Self {
            node,
            stop_count,
            stops,
        })
    }
}

impl RoutingTarget for GridPoint {
    fn target_node(&self) -> NodeIndex {
        NodeIndex::new(self.node as usize)
    }

    fn egress_stops(&self) -> impl Iterator<Item = (RaptorStopId, Time)> + '_ {
        self.stops[..self.stop_count as usize]
            .iter()
            .map(|&(stop, time)| (stop as RaptorStopId, time))
    }
}

/// Index for isochrone calculation covering a specific area
/// It contains a grid of hexagonal H3 cells and their respective
/// transit points.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsochroneIndex {
    pub grid: Vec<CellIndex>,
    points: Vec<GridPoint>,
    resolution: u8,
}

impl IsochroneIndex {
    pub fn len(&self) -> usize {
        self.grid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grid.is_empty() && self.points.is_empty()
    }

    pub fn resolution(&self) -> u8 {
        self.resolution
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            grid: Vec::new(),
            points: Vec::new(),
            resolution: 9,
        }
    }
}

impl IsochroneIndex {
    pub fn new(
        transit_model: &TransitModel,
        area: &Polygon,
        cell_resolution: u8,
        max_walking_time: Time,
    ) -> Result<Self, Error> {
        let resolution = Resolution::try_from(cell_resolution)
            .map_err(|e| Error::InvalidData(format!("Got invalid H3 resolution {e}")))?;

        let mut tiler = TilerBuilder::new(resolution)
            .containment_mode(ContainmentMode::Covers)
            .build();
        tiler.add(area.clone())?;

        let mut coverage = tiler.into_coverage();
        let mut candidates: Vec<CellIndex> = Vec::with_capacity(SNAP_CHUNK);
        let mut grid = Vec::new();
        let mut points = Vec::new();
        let mut considered = 0usize;

        loop {
            candidates.clear();
            candidates.extend(coverage.by_ref().take(SNAP_CHUNK));
            if candidates.is_empty() {
                break;
            }
            considered += candidates.len();

            let snapped: Vec<Option<GridPoint>> = candidates
                .par_iter()
                .map(|cell| GridPoint::snap(cell_centroid(*cell), transit_model, max_walking_time))
                .collect();

            for (cell, point) in candidates.iter().zip(snapped) {
                if let Some(point) = point {
                    grid.push(*cell);
                    points.push(point);
                }
            }
        }

        grid.shrink_to_fit();
        points.shrink_to_fit();

        info!(
            "Isochrone index at resolution {cell_resolution}: snapped {} of {considered} cells",
            grid.len()
        );

        Ok(Self {
            grid,
            points,
            resolution: cell_resolution,
        })
    }
}

pub fn calculate_isochrone(
    transit_model: &TransitModel,
    start_point: &TransitPoint,
    departure_time: Time,
    max_transfers: usize,
    cutoff: Time,
    index: &IsochroneIndex,
) -> Result<MultiPolygon, Error> {
    let reached_cells = compute_reachable_cells(
        transit_model,
        start_point,
        departure_time,
        max_transfers,
        cutoff,
        index,
    )?;

    let solvent = SolventBuilder::new().build();
    solvent
        .dissolve(reached_cells)
        .map_err(|e| Error::IsochroneError(e.to_string()))
}

pub fn bulk_isochrones(
    transit_model: &TransitModel,
    start_points: &[&TransitPoint],
    departure_time: Time,
    max_transfers: usize,
    cutoff: Time,
    index: &IsochroneIndex,
) -> Result<Vec<MultiPolygon>, Error> {
    let result: Result<Vec<MultiPolygon>, Error> = start_points
        .par_iter()
        .map(|start_point| {
            calculate_isochrone(
                transit_model,
                start_point,
                departure_time,
                max_transfers,
                cutoff,
                index,
            )
        })
        .collect();

    result
}

#[allow(clippy::cast_precision_loss)]
pub fn calculate_percent_access_isochrone(
    transit_model: &TransitModel,
    start_point: &TransitPoint,
    departure_range: (Time, Time),
    sample_interval: Time,
    max_transfers: usize,
    cutoff: Time,
    index: &IsochroneIndex,
) -> Result<HashMap<CellIndex, f64>, Error> {
    // Generate a list of departure times to sample
    let mut departure_times = Vec::new();
    let (start_time, end_time) = departure_range;
    let mut current_time = start_time;

    while current_time <= end_time {
        departure_times.push(current_time);
        current_time += sample_interval;
    }

    // Calculate reachable cells for each departure time
    let all_reached_cells: Result<Vec<Vec<CellIndex>>, Error> = departure_times
        .par_iter()
        .map(|&departure_time| {
            compute_reachable_cells(
                transit_model,
                start_point,
                departure_time,
                max_transfers,
                cutoff,
                index,
            )
        })
        .collect();

    // Count how many times each cell is reached
    let mut cell_access_count = HashMap::new();
    for reached_cells in &all_reached_cells? {
        for &cell in reached_cells {
            *cell_access_count.entry(cell).or_insert(0) += 1;
        }
    }

    // Calculate percentage access for each cell
    let total_samples = departure_times.len() as f64;
    let mut percent_access = HashMap::new();
    for (cell, count) in cell_access_count {
        let percentage = (f64::from(count) / total_samples) * 100.0;
        percent_access.insert(cell, percentage);
    }

    Ok(percent_access)
}

fn cell_centroid(cell: CellIndex) -> Point<f64> {
    let lat_lon = LatLng::from(cell);

    Point::new(lat_lon.lng(), lat_lon.lat())
}

fn compute_reachable_cells(
    transit_model: &TransitModel,
    start_point: &TransitPoint,
    departure_time: u32,
    max_transfers: usize,
    cutoff: u32,
    index: &IsochroneIndex,
) -> Result<Vec<CellIndex>, Error> {
    let grid = &index.grid;
    let routing_results = multimodal_routing_one_to_many(
        transit_model,
        start_point,
        &index.points,
        departure_time,
        max_transfers,
    )?;
    let reached_cells: Vec<CellIndex> = routing_results
        .iter()
        .enumerate()
        .filter_map(|(index, result)| {
            result
                .as_ref()
                .filter(|r| r.travel_time <= cutoff)
                .map(|_| grid[index])
        })
        .collect();
    Ok(reached_cells)
}

#[cfg(test)]
mod tests {
    use super::{GRID_EGRESS_STOPS, GridPoint};

    /// A national index holds millions of these, so this has to stay small.
    #[test]
    fn grid_point_stays_compact() {
        assert!(
            size_of::<GridPoint>() <= 32,
            "GridPoint grew to {} bytes",
            size_of::<GridPoint>()
        );
        assert_eq!(GRID_EGRESS_STOPS, 3);
    }
}
