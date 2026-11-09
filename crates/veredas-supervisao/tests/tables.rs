//! C-053: tables and series over portal's nested REST. The page's Tables and Series views read a
//! session's catalog, rows, listing and windows only through the session's portal router at
//! `/s/<name>/portal/`; this test reads them the same way, against a session fed by the bench and
//! a log the test writes.

mod common;

use std::time::Duration;

use common::*;
use serde_json::{Value, json};
use veredas_supervisao::{Host, config, web};

#[test]
fn a_sessions_tables_rows_series_and_windows_read_through_its_portal() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let path = write_example(dir.path(), &live_example(&bench, false));
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let host = Host::start(config::load_file(&path).unwrap()).unwrap();
    let addr = host.addr();
    let live = host.session("loop").unwrap().live();
    assert!(wait(Duration::from_secs(20), || live.health.read().unwrap().contains_key("sorter")));
    let base = store_now() - 10 * 60_000;
    append(&log, &[
        format!("{} SORT outlet=1 travel=10.5 rate=99.0", stamp(base)),
        format!("{} SORT outlet=2 travel=12.5 rate=98.0", stamp(base + 60_000)),
        format!("{} SORT outlet=1 travel=11.5 rate=97.0", stamp(base + 120_000)),
    ]);

    // The catalog: the capture rule's table, the OPC rule's, the view and the samples.
    assert!(wait(Duration::from_secs(20), || {
        let (_, t) = get(addr, "/s/loop/portal/api/tables");
        t["tables"].as_array().unwrap().iter().any(|x| x["name"] == "sorts" && x["rows"] == json!(3))
    }));
    let (status, t) = get(addr, "/s/loop/portal/api/tables");
    assert_eq!(status, 200);
    let mut names: Vec<(String, String)> = t["tables"].as_array().unwrap().iter().map(|x| (x["name"].as_str().unwrap().to_string(), x["kind"].as_str().unwrap().to_string())).collect();
    names.sort();
    assert_eq!(names.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["outlets", "plc_main", "samples", "sorts"]);
    assert_eq!(names.iter().find(|(n, _)| n == "outlets").unwrap().1, "view");
    let sorts = t["tables"].as_array().unwrap().iter().find(|x| x["name"] == "sorts").unwrap();
    let cols: Vec<&str> = sorts["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    for c in ["index_ms", "outlet", "travel", "rate"] {
        assert!(cols.contains(&c), "{cols:?}");
    }

    // A page of rows, newest first, as the Tables view pages them; the next page; a table that
    // is not there.
    let (status, p) = get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=2&offset=0");
    assert_eq!((status, p["total"].clone(), p["truncated"].clone(), p["rows"].as_array().unwrap().len()), (200, json!(3), json!(true), 2));
    let col = |p: &Value, name: &str| p["schema"].as_array().unwrap().iter().position(|c| c["name"] == name).unwrap();
    let (ti, vi) = (col(&p, "travel"), col(&p, "outlet"));
    assert_eq!(p["rows"].as_array().unwrap().iter().map(|r| (r[ti].clone(), r[vi].clone())).collect::<Vec<_>>(), [(json!(11.5), json!(1)), (json!(12.5), json!(2))]);
    let (_, p2) = get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=2&offset=2");
    assert_eq!((p2["rows"].as_array().unwrap().len(), p2["rows"][0][ti].clone(), p2["truncated"].clone()), (1, json!(10.5), json!(false)));
    assert_eq!(get(addr, "/s/loop/portal/api/table/data?name=nope"), (404, json!({ "error": "unknown table `nope`" })));

    // The listing and a window, as the Series view charts them.
    assert!(wait(Duration::from_secs(20), || {
        let (_, s) = get(addr, "/s/loop/portal/api/series");
        s["series"].as_array().unwrap().iter().any(|x| x["source"] == "sorter" && x["series"] == "travel" && x["count"] == json!(3))
    }));
    let (_, s) = get(addr, "/s/loop/portal/api/series");
    let fill1 = s["series"].as_array().unwrap().iter().find(|x| x["source"] == "plc" && x["series"] == "fill1").unwrap();
    assert_eq!((fill1["type"].clone(), fill1["last_value"].clone()), (json!("float"), json!("35")));
    let (status, w) = get(addr, &format!("/s/loop/portal/api/series/data?source=sorter&series=travel&from={}&to={}&points=600", base - 1, base + 120_000));
    assert_eq!((status, w["points"].clone(), w["total"].clone(), w["downsampled"].clone()), (200, json!([[base, 10.5], [base + 60_000, 12.5], [base + 120_000, 11.5]]), json!(3), json!(false)));

    // The page reads these through the same nested routes, and through nothing else of portal's.
    for route in ["/portal/api/tables", "/portal/api/table/data", "/portal/api/series", "/portal/api/series/data"] {
        assert!(web::APP_JS.contains(route), "{route}");
    }
    host.stop();
}
