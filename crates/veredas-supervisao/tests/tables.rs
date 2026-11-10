//! C-053: tables and series over portal's nested REST. The page's Tables and Series views read a
//! session's catalog, rows, listing and windows only through the session's portal router at
//! `/s/<name>/portal/`; this test reads them the same way, against a session fed by the bench and
//! a log the test writes, then runs the page in a headless browser and reads its two views back
//! against portal's answers, with the paths of every request it made.

mod common;

use std::time::Duration;

use common::browser::Browser;
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
    // Every route of portal's the page names is one of those, or the write it posts (C-054):
    // nothing else of portal's surface, and nothing of it outside the session's nested router.
    let mut named: Vec<String> = web::APP_JS
        .match_indices("/api/")
        .map(|(i, _)| {
            let rest = &web::APP_JS[i..];
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '/' || c == '_')).unwrap_or(rest.len());
            let before = &web::APP_JS[i.saturating_sub(7)..i];
            format!("{}{}", if before == "/portal" { "/portal" } else { "" }, &rest[..end])
        })
        .collect();
    named.sort();
    named.dedup();
    assert_eq!(named, ["/api/sessions", "/portal/api/opc/write", "/portal/api/series", "/portal/api/series/data", "/portal/api/table/data", "/portal/api/tables"]);

    // The page itself, run in a headless browser against the session. Fifty more sorts first, so
    // the Tables view has a second page.
    append(&log, &(0..50).map(|i| format!("{} SORT outlet={} travel={} rate=99.0", stamp(base + 180_000 + i * 1000), 1 + i % 3, 20.0 + i as f64 * 0.5)).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(20), || {
        let (_, t) = get(addr, "/s/loop/portal/api/tables");
        t["tables"].as_array().unwrap().iter().any(|x| x["name"] == "sorts" && x["rows"] == json!(53))
    }));
    let mut page = Browser::open(&format!("http://{addr}/s/loop/#tables"));

    // The Tables view lists the catalog as portal lists it.
    let listed = page.wait_for(r#"(() => { const b = [...document.querySelectorAll(".list button[data-table]")]; return b.length ? b.map((x) => [x.dataset.table, x.querySelector(".mono").textContent]) : null; })()"#, Duration::from_secs(20));
    let (_, t) = get(addr, "/s/loop/portal/api/tables");
    let kinds: Vec<(String, String)> = listed.as_array().unwrap().iter().map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_str().unwrap().split(" · ").next().unwrap().to_string())).collect();
    assert_eq!(kinds, t["tables"].as_array().unwrap().iter().map(|x| (x["name"].as_str().unwrap().to_string(), x["kind"].as_str().unwrap().to_string())).collect::<Vec<_>>());
    assert!(listed.as_array().unwrap().iter().any(|r| r[0] == "sorts" && r[1] == json!(format!("{} · 53", sorts["kind"].as_str().unwrap()))), "{listed}");

    // A table picked: its first page of fifty, newest first, cell for cell as portal pages it,
    // then the older page.
    const SHOWN: &str = r#"(() => {
  const pager = document.querySelector(".list-main .pager [data-rows]");
  return pager ? {
    rows: pager.dataset.rows,
    head: [...document.querySelectorAll(".list-main thead th")].map((th) => th.firstChild.textContent),
    body: [...document.querySelectorAll(".list-main tbody tr")].map((tr) => [...tr.children].map((td) => td.textContent)),
    older: [...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "Older ›")?.disabled ?? null,
  } : null;
})()"#;
    let cell = |v: &Value| match v {
        Value::Null => String::new(),
        Value::Number(n) => {
            let t = format!("{:.3}", n.as_f64().unwrap());
            let t = if t.contains('.') { t.trim_end_matches('0').trim_end_matches('.').to_string() } else { t };
            if t == "-0" { "0".to_string() } else { t }
        }
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let as_shown = |p: &Value| -> (Vec<String>, Vec<Vec<String>>) {
        (
            p["schema"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap().to_string()).collect(),
            p["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(cell).collect()).collect(),
        )
    };
    page.eval(r#"document.querySelector('.list button[data-table="sorts"]').click()"#);
    let shown = page.wait_for(&format!("(() => {{ const s = {SHOWN}; return s && s.rows === '1-50/53' ? s : null; }})()"), Duration::from_secs(20));
    let (head, body) = as_shown(&get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=50&offset=0").1);
    assert_eq!((shown["head"].clone(), shown["body"].clone(), shown["older"].clone()), (json!(head), json!(body), json!(false)));
    let travel = head.iter().position(|c| c == "travel").unwrap();
    assert_eq!(shown["body"][0][travel], json!("44.5"), "newest first");
    page.eval(r#"[...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "Older ›").click()"#);
    let shown = page.wait_for(&format!("(() => {{ const s = {SHOWN}; return s && s.rows === '51-53/53' ? s : null; }})()"), Duration::from_secs(20));
    let (_, body) = as_shown(&get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=50&offset=50").1);
    assert_eq!((shown["body"].clone(), shown["older"].clone()), (json!(body), json!(true)));
    assert_eq!(shown["body"].as_array().unwrap().iter().map(|r| r[travel].clone()).collect::<Vec<_>>(), [json!("11.5"), json!("12.5"), json!("10.5")]);

    // The Series view lists the series as portal lists them and charts a picked one's last hour,
    // every point of the window drawn in order.
    page.eval(r#"location.hash = "series""#);
    let listed = page.wait_for(r#"(() => { const b = [...document.querySelectorAll(".list button[data-series]")]; return b.length ? b.map((x) => x.dataset.series) : null; })()"#, Duration::from_secs(20));
    let (_, s) = get(addr, "/s/loop/portal/api/series");
    assert_eq!(listed, json!(s["series"].as_array().unwrap().iter().map(|x| format!("{}/{}", x["source"].as_str().unwrap(), x["series"].as_str().unwrap())).collect::<Vec<_>>()));
    page.eval(r#"document.querySelector('.list button[data-series="sorter/travel"]').click()"#);
    let chart = page.wait_for(
        r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const line = document.querySelector(".list-main svg.trend path.line"); return p && line ? { points: p.dataset.points, d: line.getAttribute("d") } : null; })()"#,
        Duration::from_secs(20),
    );
    assert_eq!(chart["points"], json!("53/53"));
    let drawn: Vec<(f64, f64)> = chart["d"]
        .as_str()
        .unwrap()
        .split(['M', 'L'])
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let mut it = p.split_whitespace().map(|n| n.parse::<f64>().unwrap());
            (it.next().unwrap(), it.next().unwrap())
        })
        .collect();
    let (_, w) = get(addr, &format!("/s/loop/portal/api/series/data?source=sorter&series=travel&from={}&to={}&points=600", base - 1, store_now()));
    let values: Vec<f64> = w["points"].as_array().unwrap().iter().map(|p| p[1].as_f64().unwrap()).collect();
    assert_eq!(drawn.len(), values.len());
    assert!(drawn.windows(2).all(|p| p[0].0 < p[1].0), "drawn in time order");
    for i in 0..values.len() {
        for j in 0..values.len() {
            if values[i] > values[j] {
                assert!(drawn[i].1 < drawn[j].1, "a higher value is drawn higher: {} > {} at {i}, {j}", values[i], values[j]);
            }
        }
    }

    // Every request the page made for these went to the session's nested portal router.
    let paths = page.eval(r#"performance.getEntriesByType("resource").map((e) => new URL(e.name).pathname)"#);
    let paths: Vec<&str> = paths.as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    for route in ["/s/loop/portal/api/tables", "/s/loop/portal/api/table/data", "/s/loop/portal/api/series", "/s/loop/portal/api/series/data"] {
        assert!(paths.contains(&route), "{route} in {paths:?}");
    }
    // (The browser asks for `/favicon.ico` on its own.)
    assert!(paths.iter().all(|p| p.starts_with("/assets/") || p.starts_with("/s/loop/") || *p == "/api/sessions" || *p == "/favicon.ico"), "{paths:?}");
    assert!(paths.iter().filter(|p| p.contains("/api/") && **p != "/api/sessions").all(|p| p.starts_with("/s/loop/portal/api/")), "{paths:?}");
    drop(page);
    host.stop();
}
