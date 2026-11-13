//! Fans, pumps and liquid temperatures from liquidctl (AIO coolers, fan
//! hubs, smart devices), polled in the background as sensors in the same
//! shape as hwmon's: {dev, label, kind: "temp"|"fan", value}.
//!
//! As the user, liquidctl sees the devices whose hidraw nodes they can open;
//! the root helper runs it as root, which reaches the rest (e.g. coolers
//! whose node is root-only). Only status is read, every few seconds.

use serde_json::{json, Value};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Vec<Value>>>;

fn liquidctl() -> Option<String> {
    std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .map(|d| std::path::Path::new(d).join("liquidctl"))
        .chain(["/run/current-system/sw/bin/liquidctl", "/usr/bin/liquidctl", "/usr/local/bin/liquidctl"].map(Into::into))
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
}

/// Runs `liquidctl status --json` (killed after a few seconds if a device
/// hangs) and turns it into sensors.
pub fn query() -> Vec<Value> {
    let Some(bin) = liquidctl() else { return Vec::new() };
    let Ok(mut child) = Command::new(bin)
        .args(["status", "--json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    }
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = std::io::Read::read_to_string(&mut s, &mut out);
    }
    parse(&out)
}

pub fn parse(text: &str) -> Vec<Value> {
    let Ok(Value::Array(devices)) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut out = Vec::new();
    for d in devices {
        let dev = d["description"].as_str().unwrap_or("liquidctl device").to_string();
        for st in d["status"].as_array().into_iter().flatten() {
            let key = st["key"].as_str().unwrap_or("");
            let unit = st["unit"].as_str().unwrap_or("");
            let Some(value) = st["value"].as_f64() else { continue };
            let (kind, label) = match unit {
                "rpm" => ("fan", key.trim_end_matches(" speed").to_string()),
                "°C" => ("temp", key.trim_end_matches(" temperature").to_string()),
                _ => continue,
            };
            // A 0 rpm channel is an empty header; leave it out.
            if kind == "fan" && value <= 0.0 {
                continue;
            }
            out.push(json!({"dev": dev, "label": label, "kind": kind, "value": value}));
        }
    }
    out
}

/// lights-out (an MSI CORELIQUID controller, since liquidctl no longer
/// accepts that cooler) publishes its fan/pump readings here.
const LIGHTS_OUT: &str = "/run/lights-out/cooler.json";

/// The cooler's fans and pump from lights-out, if it published recently.
pub fn lights_out() -> Vec<Value> {
    let Ok(text) = std::fs::read_to_string(LIGHTS_OUT) else { return Vec::new() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    parse_lights_out(&text, now)
}

pub fn parse_lights_out(text: &str, now: u64) -> Vec<Value> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    if now.saturating_sub(v["updated"].as_u64().unwrap_or(0)) > 30 {
        return Vec::new(); // stale: the daemon stopped
    }
    let dev = v["device"].as_str().unwrap_or("Cooler").to_string();
    v["channels"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|c| c["rpm"].as_f64().unwrap_or(0.0) > 0.0)
        .map(|c| {
            let duty = c["duty"].as_u64().map(|d| format!(" {d}%")).unwrap_or_default();
            json!({"dev": dev, "label": format!("{}{duty}", c["name"].as_str().unwrap_or("fan")), "kind": "fan", "value": c["rpm"].as_f64().unwrap_or(0.0)})
        })
        .collect()
}

/// Polls liquidctl every few seconds on its own thread.
pub fn spawn_poller() -> Shared {
    let shared: Shared = Arc::new(Mutex::new(Vec::new()));
    let out = shared.clone();
    std::thread::Builder::new()
        .name("treetop-liquidctl".into())
        .spawn(move || {
            if liquidctl().is_none() {
                return;
            }
            loop {
                let v = query();
                *out.lock().unwrap() = v;
                std::thread::sleep(Duration::from_secs(5));
            }
        })
        .ok();
    shared
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_lights_out() {
        // Exactly what lights-out's publish_cooler() writes.
        let t = r#"{"device":"MSI MPG CORELIQUID","mode":"quiet","cooling":"Quiet","cpu":63.4,"updated":1000,"channels":[{"name":"Radiator fan 1","rpm":1100,"duty":40},{"name":"Radiator fan 2","rpm":0,"duty":0},{"name":"Pump","rpm":2400,"duty":60}]}"#;
        let v = super::parse_lights_out(t, 1005);
        let got: Vec<_> = v.iter().map(|x| (x["label"].as_str().unwrap().to_string(), x["value"].as_f64().unwrap())).collect();
        assert_eq!(got, vec![("Radiator fan 1 40%".to_string(), 1100.0), ("Pump 60%".to_string(), 2400.0)]);
        assert!(super::parse_lights_out(t, 1100).is_empty(), "stale file ignored");
    }

    #[test]
    fn parses_liquidctl_status() {
        let s = r#"[{"bus":"hid","address":"/dev/hidraw1","description":"Lian Li Uni AL V2","status":[
            {"key":"Fan 1 speed","value":0,"unit":"rpm"},{"key":"Fan 2 speed","value":255,"unit":"rpm"}]},
            {"description":"MSI MPG Coreliquid","status":[{"key":"Pump speed","value":2400,"unit":"rpm"},
            {"key":"Liquid temperature","value":34.5,"unit":"°C"},{"key":"Pump duty","value":60,"unit":"%"}]}]"#;
        let v = super::parse(s);
        let labels: Vec<_> = v.iter().map(|x| (x["dev"].as_str().unwrap(), x["label"].as_str().unwrap(), x["kind"].as_str().unwrap())).collect();
        assert_eq!(labels, vec![
            ("Lian Li Uni AL V2", "Fan 2", "fan"),
            ("MSI MPG Coreliquid", "Pump", "fan"),
            ("MSI MPG Coreliquid", "Liquid", "temp"),
        ]);
    }
}
