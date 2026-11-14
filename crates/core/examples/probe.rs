//! Prints a few engine updates' headline numbers (a quick sanity check).
fn main() {
    let e = treetop_core::Engine::new();
    let (tx, rx) = std::sync::mpsc::channel();
    e.start(move |v| tx.send(v).is_ok());
    for _ in 0..4 {
        let v = rx.recv().unwrap();
        if v["rowsOnly"].as_bool() == Some(true) { continue; }
        println!("cpu {:?} mem {:?} rx {:?} disk {:?} rows {:?}", v["cpu"]["total"], v["mem"]["used"], v["net"]["rx"], v["disk"]["read"], v["rows"]["count"]);
    }
}
