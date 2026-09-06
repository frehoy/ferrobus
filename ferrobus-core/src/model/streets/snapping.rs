//! Projection and deterministic splitting of pedestrian polylines.
use std::collections::BTreeMap;

use geo::{Distance, Haversine, Point};
use petgraph::graph::{EdgeIndex, NodeIndex};
use rstar::{
    RTree,
    primitives::{GeomWithData, Line, Rectangle},
};
use serde::{Deserialize, Serialize};

use super::{StreetEdge, StreetGraph, StreetNode};
use crate::{Time, WALKING_SPEED};

// Unit-sphere chords avoid the latitude bias of longitude/latitude distances.
pub(super) type IndexedEdge = GeomWithData<Rectangle<[f64; 3]>, usize>;

fn xyz(point: Point<f64>) -> [f64; 3] {
    let (lon, lat) = (point.x().to_radians(), point.y().to_radians());
    [lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin()]
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn seconds(distance: f64) -> Time {
    // Suppress floating-point residue when a point lies on the polyline.
    if distance < 1e-6 {
        0
    } else {
        (distance / WALKING_SPEED).ceil() as Time
    }
}

/// A virtual point on a street edge. Costs include the off-street connection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StreetSnap {
    pub edge: usize,
    pub segment: usize,
    pub point: Point<f64>,
    pub nodes: [NodeIndex; 2],
    pub costs: [Time; 2],
    pub access: Time,
    /// Cumulative cost from the source; differences conserve edge weight.
    pub offset: Time,
    pub fraction: f64,
}

/// Compact attachment retained by query points and H3 cells.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct StreetLocation {
    pub edge: u32,
    pub nodes: [NodeIndex; 2],
    pub costs: [Time; 2],
    pub offset: Time,
    pub access: Time,
}

impl StreetSnap {
    pub(crate) fn location(self) -> StreetLocation {
        StreetLocation {
            edge: u32::try_from(self.edge).expect("u32 edge index"),
            nodes: self.nodes,
            costs: self.costs,
            offset: self.offset,
            access: self.access,
        }
    }
}

impl StreetGraph {
    pub(super) fn build_edge_index(&self) -> RTree<IndexedEdge> {
        RTree::bulk_load(
            self.graph
                .edge_indices()
                .filter_map(|edge| {
                    let geometry = &self.graph[edge].geometry;
                    if geometry.len() < 2 {
                        return None;
                    }
                    let mut lower = [f64::INFINITY; 3];
                    let mut upper = [f64::NEG_INFINITY; 3];
                    for point in geometry {
                        let coordinate = xyz(*point);
                        for axis in 0..3 {
                            lower[axis] = lower[axis].min(coordinate[axis]);
                            upper[axis] = upper[axis].max(coordinate[axis]);
                        }
                    }
                    Some(GeomWithData::new(
                        Rectangle::from_corners(lower, upper),
                        edge.index(),
                    ))
                })
                .collect(),
        )
    }

    /// Bounding-box distance is a lower bound on every chord in the polyline.
    /// Search until no remaining edge can improve the exact segment distance.
    fn nearest_segment(&self, query: [f64; 3]) -> Option<(usize, usize)> {
        use rstar::PointDistance;
        let mut distance = f64::INFINITY;
        let mut best = None;
        for candidate in self.edge_rtree.nearest_neighbor_iter(&query) {
            if candidate.distance_2(&query) > distance {
                break;
            }
            let edge_id = candidate.data;
            for (segment, pair) in self.graph[EdgeIndex::new(edge_id)]
                .geometry
                .windows(2)
                .enumerate()
            {
                let cost = Line::new(xyz(pair[0]), xyz(pair[1])).distance_2(&query);
                let key = (edge_id, segment);
                if cost
                    .total_cmp(&distance)
                    .then_with(|| Some(key).cmp(&best))
                    .is_lt()
                {
                    distance = cost;
                    best = Some(key);
                }
            }
        }
        best
    }

    /// Project onto the nearest polyline segment, with stable ties at junctions.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::float_cmp
    )]
    pub(crate) fn nearest_edge(&self, point: Point<f64>) -> Option<StreetSnap> {
        let query = xyz(point);
        let (edge_id, segment) = self.nearest_segment(query)?;
        let edge_index = EdgeIndex::new(edge_id);
        let edge = &self.graph[edge_index];
        let (source, target) = self.graph.edge_endpoints(edge_index)?;
        let a = xyz(edge.geometry[segment]);
        let b = xyz(edge.geometry[segment + 1]);
        let delta = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let chord_squared: f64 = delta.iter().map(|x| x * x).sum();
        let t = if chord_squared == 0.0 {
            0.0
        } else {
            ((0..3).map(|i| (query[i] - a[i]) * delta[i]).sum::<f64>() / chord_squared)
                .clamp(0.0, 1.0)
        };
        let projected = [
            a[0] + t * delta[0],
            a[1] + t * delta[1],
            a[2] + t * delta[2],
        ];
        let projection = if t == 0.0 {
            edge.geometry[segment]
        } else if t == 1.0 {
            edge.geometry[segment + 1]
        } else {
            Point::new(
                projected[1].atan2(projected[0]).to_degrees(),
                projected[2]
                    .atan2(projected[0].hypot(projected[1]))
                    .to_degrees(),
            )
        };
        let lengths: Vec<_> = edge
            .geometry
            .windows(2)
            .map(|p| Haversine.distance(p[0], p[1]))
            .collect();
        let total: f64 = lengths.iter().sum();
        let along = lengths[..segment].iter().sum::<f64>()
            + Haversine.distance(edge.geometry[segment], projection);
        let fraction = if total == 0.0 {
            0.0
        } else {
            (along / total).clamp(0.0, 1.0)
        };
        let offset = (f64::from(edge.weight) * fraction).round() as Time;
        let access = seconds(Haversine.distance(point, projection));
        Some(StreetSnap {
            edge: edge_id,
            segment,
            point: projection,
            nodes: [source, target],
            costs: [
                access.saturating_add(offset),
                access.saturating_add(edge.weight - offset),
            ],
            access,
            offset,
            fraction,
        })
    }

    /// Split every affected edge once, then add explicit off-street stop links.
    /// Input order determines synthetic node IDs; edge order determines splits.
    // Fractions are clamped; exact endpoints must reuse the junction.
    #[allow(clippy::float_cmp)]
    pub(crate) fn link_stops(
        &mut self,
        points: &[Point<f64>],
        budget: Time,
    ) -> Vec<Option<NodeIndex>> {
        let mut groups: BTreeMap<usize, Vec<(usize, StreetSnap)>> = BTreeMap::new();
        for (id, &point) in points.iter().enumerate() {
            if let Some(snap) = self.nearest_edge(point).filter(|s| s.access <= budget) {
                groups.entry(snap.edge).or_default().push((id, snap));
            }
        }
        let mut result = vec![None; points.len()];
        // Removing backwards keeps all not-yet-processed edge indices valid.
        for (edge_id, mut snaps) in groups.into_iter().rev() {
            snaps.sort_by(|a, b| a.1.fraction.total_cmp(&b.1.fraction).then(a.0.cmp(&b.0)));
            let index = EdgeIndex::new(edge_id);
            let (source, target) = self.graph.edge_endpoints(index).expect("indexed edge");
            let edge = self.graph.remove_edge(index).expect("indexed edge");
            let mut connectors = BTreeMap::new();
            let mut previous_node = source;
            let mut previous_offset = 0;
            let mut previous_segment = 0;
            let mut previous_point = edge.geometry[0];
            for (stop_id, snap) in snaps {
                let projection_node = if snap.fraction == 0.0 {
                    source
                } else if snap.fraction == 1.0 {
                    target
                } else if snap.point == previous_point {
                    previous_node
                } else {
                    self.synthetic_node(snap.point)
                };
                if projection_node != previous_node {
                    let mut geometry = vec![previous_point];
                    geometry.extend_from_slice(&edge.geometry[previous_segment + 1..=snap.segment]);
                    geometry.push(snap.point);
                    self.graph.add_edge(
                        previous_node,
                        projection_node,
                        StreetEdge {
                            weight: snap.offset - previous_offset,
                            geometry,
                        },
                    );
                }
                let stop_node = if snap.access == 0 {
                    projection_node
                } else {
                    let key = (
                        projection_node.index(),
                        points[stop_id].x().to_bits(),
                        points[stop_id].y().to_bits(),
                    );
                    *connectors.entry(key).or_insert_with(|| {
                        let node = self.synthetic_node(points[stop_id]);
                        // One coordinate excludes artificial connectors from the snap index.
                        self.graph.add_edge(
                            projection_node,
                            node,
                            StreetEdge {
                                weight: snap.access,
                                geometry: vec![points[stop_id]],
                            },
                        );
                        node
                    })
                };
                result[stop_id] = Some(stop_node);
                previous_node = projection_node;
                previous_offset = snap.offset;
                previous_segment = snap.segment;
                previous_point = snap.point;
            }
            if previous_node != target || previous_offset < edge.weight {
                let mut geometry = vec![previous_point];
                geometry.extend_from_slice(&edge.geometry[previous_segment + 1..]);
                self.graph.add_edge(
                    previous_node,
                    target,
                    StreetEdge {
                        weight: edge.weight - previous_offset,
                        geometry,
                    },
                );
            }
        }
        self.rtree = crate::loading::build_rtree(&self.graph);
        self.edge_rtree = self.build_edge_index();
        result
    }

    fn synthetic_node(&mut self, geometry: Point<f64>) -> NodeIndex {
        let id = -1 - i64::try_from(self.graph.node_count()).expect("u32 graph index fits i64");
        self.graph.add_node(StreetNode {
            id: osm4routing::NodeId(id),
            geometry,
        })
    }
}
