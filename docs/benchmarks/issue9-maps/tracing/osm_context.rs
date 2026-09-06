use std::{collections::HashMap,env,fs};
use serde_json::{Value,json};
fn main()->Result<(),Box<dyn std::error::Error>> {
 let args:Vec<_>=env::args().collect();
 let queries:Vec<Value>=serde_json::from_slice(&fs::read(&args[2])?)?;
 let boxes:Vec<_>=queries.iter().map(|r|{
  let c=r["coordinates"].as_array().unwrap();let x=c[0].as_f64().unwrap();let y=c[1].as_f64().unwrap();
  let dy=2500./111320.;let dx=dy/y.to_radians().cos();(x-dx,y-dy,x+dx,y+dy)
 }).collect();
 let inside=|x:f64,y:f64|boxes.iter().any(|&(l,b,r,t)|x>=l&&x<=r&&y>=b&&y<=t);
 let mut nodes=HashMap::new();let mut ways=Vec::new();
 osmpbf::ElementReader::from_path(&args[1])?.for_each(|e|match e {
  osmpbf::Element::Node(n)=>{if inside(n.lon(),n.lat()){nodes.insert(n.id(),[n.lon(),n.lat()]);}},
  osmpbf::Element::DenseNode(n)=>{if inside(n.lon(),n.lat()){nodes.insert(n.id(),[n.lon(),n.lat()]);}},
  osmpbf::Element::Way(w)=>{
   if !w.refs().any(|r|nodes.contains_key(&r)){return;}
   let tags:HashMap<_,_>=w.tags().collect();
   let points:Vec<_>=w.refs().map(|id|json!({"id":id,"xy":nodes.get(&id)})).collect();
   ways.push(json!({"id":w.id(),"tags":tags,"nodes":points}));
  },_=>{}
 })?;
 eprintln!("{} nodes, {} ways",nodes.len(),ways.len());
 println!("{}",serde_json::to_string(&ways)?);Ok(())
}
