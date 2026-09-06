use ferrobus_core::{TransitModel,TransitPoint,load_transit_model};
use geo::Point;
use petgraph::{graph::{NodeIndex,EdgeIndex},visit::EdgeRef};
use serde_json::{json,Value};
use std::{collections::{BinaryHeap,HashMap},cmp::Reverse,env,fs};

struct Attach { nodes:[NodeIndex;2], costs:[u32;2], edge:Option<usize>, offset:u32, point:Point<f64>, segment:usize, access:u32 }
fn xy(p:Point<f64>)->[f64;2] {[p.x(),p.y()]}
fn attach(model:&TransitModel,p:Point<f64>)->Attach {
 let s=model.street_graph.nearest_edge(p).unwrap();
 Attach {nodes:s.nodes,costs:s.costs,edge:Some(s.edge),offset:s.offset,point:s.point,segment:s.segment,access:s.access}
}
fn edge_geometry(model:&TransitModel,e:EdgeIndex,from:NodeIndex)->Vec<[f64;2]> {
 let (source,target)=model.street_graph.graph.edge_endpoints(e).unwrap();
 let mut points:Vec<_>=model.street_graph.graph[e].geometry.iter().copied().map(xy).collect();
 if points.len()<2 {points=vec![xy(model.street_graph.graph[source].geometry),xy(model.street_graph.graph[target].geometry)];}
 if source!=from {points.reverse();} points
}
fn side_path(model:&TransitModel,a:&Attach,node:NodeIndex)->Vec<[f64;2]> {
    let Some(edge)=a.edge else { return vec![xy(a.point)]; };
    let e=EdgeIndex::new(edge);
    let (source,_)=model.street_graph.graph.edge_endpoints(e).unwrap();
    let geom=edge_geometry(model,e,source);
    let mut path=vec![xy(a.point)];
    if node==a.nodes[0] {path.extend(geom[..=a.segment].iter().rev());}
    else {path.extend_from_slice(&geom[a.segment+1..]);}
    path
}
fn main()->Result<(),Box<dyn std::error::Error>> {
    let args:Vec<_>=env::args().collect();
    let model=load_transit_model(&args[1])?;
    let rows:Vec<Value>=serde_json::from_slice(&fs::read(&args[2])?)?;
    let mut results=Vec::new();
    for row in rows {
        let c:Vec<f64>=row["coordinates"].as_array().unwrap().iter().map(|v|v.as_f64().unwrap()).collect();
        let from=Point::new(c[0],c[1]);let to=Point::new(c[2],c[3]);
        let a=attach(&model,from);let b=attach(&model,to);
        let mut dist=HashMap::new();let mut prev=HashMap::new();let mut heap=BinaryHeap::new();
        for (node,cost) in a.nodes.into_iter().zip(a.costs) {
            if dist.get(&node).is_none_or(|&v|cost<v) { dist.insert(node,cost); heap.push(Reverse((cost,node.index()))); }
        }
        while let Some(Reverse((cost,id)))=heap.pop() {
            let node=NodeIndex::new(id);
            if cost>1200 || cost>dist[&node] {continue;}
            for edge in model.street_graph.graph.edges(node) {
                let next=edge.target();let value=cost+edge.weight().weight;
                if value<=1200 && dist.get(&next).is_none_or(|&v|value<v) {
                    dist.insert(next,value);prev.insert(next,(node,edge.id()));heap.push(Reverse((value,next.index())));
                }
            }
        }
        let best=b.nodes.into_iter().zip(b.costs).filter_map(|(n,c)|dist.get(&n).map(|v|(*v+c,n))).min();
        let same=if a.edge.is_some()&&a.edge==b.edge {Some(a.offset.abs_diff(b.offset)+a.access+b.access)} else {None};
        let mut paths:Vec<Vec<[f64;2]>>=Vec::new();let mut route_edges=Vec::new();let mut route_nodes=Vec::new();
        let seconds;
        if same.is_some_and(|v|best.is_none_or(|(cost,_)|v<=cost)) {
            seconds=same.unwrap();
            let edge=EdgeIndex::new(a.edge.unwrap());let source=model.street_graph.graph.edge_endpoints(edge).unwrap().0;
            let geom=edge_geometry(&model,edge,source);
            let (low,high,reverse)=if a.offset<=b.offset {(&a,&b,false)} else {(&b,&a,true)};
            let mut path=vec![xy(low.point)];path.extend_from_slice(&geom[low.segment+1..=high.segment]);path.push(xy(high.point));
            if reverse {path.reverse();} paths.push(path);
        } else {
            let (cost,mut node)=best.ok_or("no path")?;seconds=cost;
            let target=node;let mut edges=Vec::new();route_nodes.push(node.index());
            while let Some(&(parent,edge))=prev.get(&node) {edges.push((parent,node,edge));node=parent;route_nodes.push(node.index());}
            edges.reverse();route_nodes.reverse();
            paths.push(side_path(&model,&a,node));
            for (source,target,edge) in edges {
                let geometry=edge_geometry(&model,edge,source);
                route_edges.push(json!({"source_osm":model.street_graph.graph[source].id.0,"target_osm":model.street_graph.graph[target].id.0,"weight":model.street_graph.graph[edge].weight,"geometry":geometry}));
                paths.push(geometry);
            }
            let mut end=side_path(&model,&b,target);end.reverse();paths.push(end);
        }
        let expected=TransitPoint::new(from,&model,1200,3)?.walking_time_to(&TransitPoint::new(to,&model,1200,3)?);
        assert_eq!(Some(seconds),expected,"tracer must match production cost");
        results.push(json!({"id":row["id"],"coordinates":c,"seconds":seconds,"from_snap":xy(a.point),"to_snap":xy(b.point),"from_access_seconds":a.access,"to_access_seconds":b.access,"from_edge":a.edge,"to_edge":b.edge,"paths":paths,"route_edges":route_edges,"route_nodes":route_nodes}));
    }
    println!("{}",serde_json::to_string_pretty(&results)?);Ok(())
}
