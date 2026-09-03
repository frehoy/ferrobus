use hashbrown::HashMap;
#[cfg(test)]
use hashbrown::HashSet;
use log::info;
use osm4routing::FootAccessibility;
use petgraph::graph::{NodeIndex, UnGraph};
use petgraph::unionfind::UnionFind;
use petgraph::visit::EdgeRef;
use rstar::RTree;
use std::path::Path;

use crate::{
    Error, Time, WALKING_SPEED,
    model::{IndexedPoint, StreetEdge, StreetGraph, StreetNode},
};

/// Discard everything outside the largest connected component, in place.
fn keep_largest_component(graph: &mut UnGraph<StreetNode, StreetEdge>) -> Result<(), Error> {
    let node_count = graph.node_count();

    let mut components = UnionFind::<u32>::new(node_count);
    for edge in graph.edge_references() {
        let (source, target) = (edge.source().index(), edge.target().index());
        // The graph is built from `NodeIndex<u32>`, so both fit by construction.
        #[allow(clippy::cast_possible_truncation)]
        components.union(source as u32, target as u32);
    }

    // Labels are representative node indices, so they index a plain counter.
    let labels = components.into_labeling();
    let mut sizes = vec![0u32; node_count];
    for &label in &labels {
        sizes[label as usize] += 1;
    }

    let largest = sizes
        .iter()
        .enumerate()
        .max_by_key(|&(_, count)| count)
        .filter(|&(_, &count)| count > 0)
        .map(|(label, _)| label)
        .ok_or_else(|| Error::InvalidData("No connected components found".to_string()))?;
    drop(sizes);

    // High to low: `remove_node` swaps the last node into the hole, and every
    // index above this one has already been decided, so `labels[index]` still
    // describes the node under `index`.
    for index in (0..node_count).rev() {
        if labels[index] as usize != largest {
            graph.remove_node(NodeIndex::new(index));
        }
    }
    graph.shrink_to_fit();

    Ok(())
}

/// Create the street network graph based on an OSM .pbf file
pub(crate) fn create_street_graph(filename: impl AsRef<Path>) -> Result<StreetGraph, Error> {
    info!("Reading OSM data from: {}", filename.as_ref().display());

    let mut graph = UnGraph::<StreetNode, StreetEdge>::new_undirected();
    // Store OSM node IDs and their corresponding graph node indices
    // No tag filter here: osm4routing already drops what it cannot walk.
    let (nodes, edges) = osm4routing::Reader::new()
        .read(filename)
        .map_err(|e| Error::InvalidData(format!("Error reading OSM data: {e}")))?;
    info!("OSM read: {} nodes, {} ways", nodes.len(), edges.len());

    // Only a way's length is read, so the fat `Edge` values are dropped here.
    let edges: Vec<(osm4routing::NodeId, osm4routing::NodeId, Time)> = edges
        .into_iter()
        .filter(|edge| edge.properties.foot == FootAccessibility::Allowed)
        // A way whose ends are the same node is a loop nothing can route over.
        .filter(|edge| edge.source != edge.target)
        .map(|edge| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let weight = (edge.length() / WALKING_SPEED) as Time;
            (edge.source, edge.target, weight)
        })
        .collect();
    info!("Kept {} pedestrian ways", edges.len());

    let mut node_indices = HashMap::new();

    for node in nodes {
        node_indices.entry(node.id).or_insert_with(|| {
            let node_obj = StreetNode {
                id: node.id,
                geometry: node.coord.into(),
            };

            graph.add_node(node_obj)
        });
    }

    info!("Indexed {} distinct OSM nodes", node_indices.len());

    for (source, target, weight) in edges {
        let source_index = *node_indices
            .get(&source)
            .ok_or_else(|| Error::InvalidData(format!("Missing source node: {source:?}")))?;
        let target_index = *node_indices
            .get(&target)
            .ok_or_else(|| Error::InvalidData(format!("Missing target node: {target:?}")))?;

        graph.add_edge(source_index, target_index, StreetEdge { weight });
    }

    // One entry per OSM node, and nothing below reads it.
    drop(node_indices);
    info!(
        "Graph assembled: {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    // Keep only the largest connected component to avoid isolated parts of the graph
    // affecting routing
    info!("Pruning to the largest connected component");
    keep_largest_component(&mut graph)?;
    info!(
        "Street network pruned to {} nodes, {} edges",
        graph.node_count(),
        graph.edge_count()
    );

    info!("Building R-Tree spatial index");
    let rtree = build_rtree(&graph);

    let street_network = StreetGraph { graph, rtree };

    Ok(street_network)
}

/// R*-tree spatial index for quick nearest neighbor queries
pub(crate) fn build_rtree(graph: &UnGraph<StreetNode, StreetEdge>) -> RTree<IndexedPoint> {
    let mut points = Vec::with_capacity(graph.node_count());
    for (idx, node) in graph.node_weights().enumerate() {
        let idx = NodeIndex::new(idx);
        points.push(IndexedPoint::new(node.geometry, idx));
    }
    RTree::bulk_load(points)
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Point;
    use osm4routing::NodeId;

    #[test]
    fn keeps_the_largest_component_and_drops_the_rest() {
        let node = |id, x: f64| StreetNode {
            id: NodeId(id),
            geometry: Point::new(x, 0.0),
        };
        let mut graph = UnGraph::<StreetNode, StreetEdge>::new_undirected();
        // A three-node path, plus an unrelated pair that must not survive.
        let n0 = graph.add_node(node(1, 0.0));
        let n1 = graph.add_node(node(2, 1.0));
        let n2 = graph.add_node(node(3, 2.0));
        let n3 = graph.add_node(node(4, 10.0));
        let n4 = graph.add_node(node(5, 11.0));
        graph.add_edge(n0, n1, StreetEdge { weight: 10 });
        graph.add_edge(n1, n2, StreetEdge { weight: 20 });
        graph.add_edge(n3, n4, StreetEdge { weight: 30 });

        keep_largest_component(&mut graph).expect("a graph with edges has a component");

        assert_eq!(graph.node_count(), 3);
        // Pruning in place must not shed or double the surviving edges.
        assert_eq!(graph.edge_count(), 2);
        let kept: HashSet<i64> = graph.node_weights().map(|n| n.id.0).collect();
        assert_eq!(kept, HashSet::from([1, 2, 3]));
    }
}
