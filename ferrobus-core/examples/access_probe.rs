//! Comparable JSONL walking-access samples from two builds of the same dataset.
//! Run each build's binary against its own model: access_probe <model> [queries.json].
//! Optional queries are [[from_lon,from_lat,to_lon,to_lat], ...]. Without a file,
//! sample four roughly 50m offsets around 100 evenly spaced transit stops.
use ferrobus_core::{TransitPoint, load_transit_model, multimodal_routing};
use geo::Point;
use serde_json::json;
use std::{env, fs, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    let model = load_transit_model(
        args.get(1)
            .ok_or("usage: access_probe <model> [queries.json]")?,
    )?;
    let queries: Vec<[f64; 4]> = if let Some(path) = args.get(2) {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        model
            .stops()
            .iter()
            .step_by(model.stop_count().div_ceil(100).max(1))
            .flat_map(|stop| {
                let p = stop.geometry;
                let dy = 50.0 / 111_320.0;
                let dx = dy / p.y().to_radians().cos();
                [(dx, 0.), (-dx, 0.), (0., dy), (0., -dy)]
                    .map(|(x, y)| [p.x() + x, p.y() + y, p.x(), p.y()])
            })
            .collect()
    };
    for (id, coords) in queries.iter().enumerate() {
        let started = Instant::now();
        let from = TransitPoint::new(Point::new(coords[0], coords[1]), &model, 1200, 3);
        let to = TransitPoint::new(Point::new(coords[2], coords[3]), &model, 1200, 3);
        let record = match (from, to) {
            (Ok(from), Ok(to)) => {
                let walk = from.walking_time_to(&to);
                let route = multimodal_routing(&model, &from, &to, 28800, 3)?;
                json!({"id":id,"coordinates":coords,"walking_seconds":walk,
                    "from_stops":from.nearest_stops(),"to_stops":to.nearest_stops(),
                    "travel_seconds":route.as_ref().map(|r|r.travel_time),
                    "journey_walking_seconds":route.as_ref().map(|r|r.walking_time),
                    "query_us":started.elapsed().as_micros()})
            }
            _ => {
                json!({"id":id,"coordinates":coords,"walking_seconds":null,"travel_seconds":null,"error":"cannot snap within 1200s","query_us":started.elapsed().as_micros()})
            }
        };
        println!("{record}");
    }
    Ok(())
}
