//! Street network components - nodes, edges, and transit points

use geo::Point;
pub use osm4routing::NodeId;
use serde::{Deserialize, Serialize};

use crate::Time;

/// Street graph node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreetNode {
    /// OSM ID of the node
    #[serde(with = "node_id_serde")]
    pub id: NodeId,
    /// Node coordinates
    pub geometry: Point<f64>,
}

/// `osm4routing::NodeId` is a foreign newtype without serde support,
/// so it is persisted as the plain OSM identifier it wraps.
mod node_id_serde {
    use super::NodeId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    // serde's `with` contract fixes this signature; the reference is not ours to drop.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub(super) fn serialize<S: Serializer>(id: &NodeId, ser: S) -> Result<S::Ok, S::Error> {
        id.0.serialize(ser)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<NodeId, D::Error> {
        i64::deserialize(de).map(NodeId)
    }
}

/// Street graph edge (street segment)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreetEdge {
    /// Pedestrian crossing time in seconds
    pub weight: Time,
}

impl StreetEdge {
    pub fn walking_time(&self) -> Time {
        self.weight
    }
}
