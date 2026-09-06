//! Walkable network model and related functions

use geo::{Distance, Haversine, Point};
use petgraph::graph::{EdgeReference, NodeIndex, UnGraph};
use rstar::{RTree, primitives::GeomWithData};
use serde::{Deserialize, Serialize};

use super::components::{StreetEdge, StreetNode};
use crate::WALKING_SPEED;

/// Struct for storing graph node in R-tree index
pub type IndexedPoint = GeomWithData<Point<f64>, NodeIndex>;

/// Pedestrian network model based on OSM data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreetGraph {
    /// Street graph
    pub graph: UnGraph<StreetNode, StreetEdge>,
    /// Spatial index for fast nearest node search
    pub rtree: RTree<IndexedPoint>,
    pub(super) edge_rtree: RTree<super::snapping::IndexedEdge>,
}

impl StreetGraph {
    /// Construct spatial indexes, using endpoint geometry for hand-built edges.
    pub fn new(mut graph: UnGraph<StreetNode, StreetEdge>) -> Self {
        for id in graph.edge_indices().collect::<Vec<_>>() {
            if graph[id].geometry.is_empty()
                && let Some((a, b)) = graph.edge_endpoints(id)
            {
                graph[id].geometry = vec![graph[a].geometry, graph[b].geometry];
            }
        }
        let rtree = crate::loading::build_rtree(&graph);
        let mut result = Self {
            graph,
            rtree,
            edge_rtree: RTree::new(),
        };
        result.edge_rtree = result.build_edge_index();
        result
    }

    pub(crate) fn edges(
        &self,
        node: NodeIndex,
    ) -> impl Iterator<Item = EdgeReference<'_, StreetEdge, u32>> {
        self.graph.edges(node)
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(crate) fn nearest_node(&self, point: &Point<f64>) -> Option<(NodeIndex, u32)> {
        self.rtree.nearest_neighbor(point).map(|indexed_point| {
            let distance =
                (Haversine.distance(*point, *indexed_point.geom()) / WALKING_SPEED).ceil() as u32;
            (indexed_point.data, distance)
        })
    }
}

#[cfg(test)]
mod tests {
    use osm4routing::NodeId;

    use super::*;

    #[test]
    fn test_nearest_node() {
        let mut graph = UnGraph::new_undirected();
        let a = graph.add_node(StreetNode {
            id: NodeId(0i64),
            geometry: Point::new(0.0, 0.0),
        });
        let b = graph.add_node(StreetNode {
            id: NodeId(1i64),
            geometry: Point::new(1.0, 1.0),
        });
        let c = graph.add_node(StreetNode {
            id: NodeId(2i64),
            geometry: Point::new(2.0, 2.0),
        });

        let network = StreetGraph::new(graph);

        let (node, _) = network.nearest_node(&Point::new(0.4, 0.4)).unwrap();
        assert_eq!(node, a);

        let (node, _) = network.nearest_node(&Point::new(1.4, 1.4)).unwrap();
        assert_eq!(node, b);

        let (node, _) = network.nearest_node(&Point::new(2.5, 2.5)).unwrap();
        assert_eq!(node, c);
    }
}
