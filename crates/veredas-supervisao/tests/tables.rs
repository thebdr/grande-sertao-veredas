//! C-053: tables and series over portal's nested REST. The page's Tables and Series views read a
//! session's catalog, rows, listing and windows only through the session's portal router at
//! `/s/<name>/portal/`; this test reads them the same way, against a session fed by the bench and
//! a log the test writes, then runs the page in a headless browser and reads its two views back
//! against portal's answers (every button's count, a table's pages whichever table was paged
//! before, the axes' labels where they sit, a window of one point), with the paths of every
//! request it made, WebSockets among them, in both sessions.

mod common;

use std::time::Duration;

use common::browser::Browser;
use common::*;
use serde_json::{Value, json};
use veredas_supervisao::{Host, config, web};

#[test]
fn a_sessions_tables_rows_series_and_windows_read_through_its_portal() {
    // Two plants on one host, so the page's reading of its own session is told from another's.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let other = plant(&dir.path().join("bench-hall"));
    let path = two_plants(dir.path(), &bench, &other);
    let log = dir.path().join("loop/sorts.log");
    std::fs::write(&log, "").unwrap();
    let hall_log = dir.path().join("hall/sorts.log");
    std::fs::write(&hall_log, "").unwrap();
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

    // Every route of portal's the bundle names sits under the session's nested router and is one
    // of the four or the write: no other `/portal/` path (portal's event stream among them).
    let portal: Vec<&str> = web::APP_JS
        .match_indices("/portal")
        .map(|(i, _)| {
            let rest = &web::APP_JS[i..];
            &rest[..rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '/' || c == '_')).unwrap_or(rest.len())]
        })
        .collect();
    assert!(portal.iter().all(|p| ["/portal/api/tables", "/portal/api/table/data", "/portal/api/series", "/portal/api/series/data", "/portal/api/opc/write"].contains(p)), "{portal:?}");

    // The page itself, run in a headless browser against the session, every request it makes
    // watched from its first (streams and other origins included). Fifty more sorts first, so the
    // Tables view has a second page; the oldest of them to three places.
    append(&log, &(0..50).map(|i| format!("{} SORT outlet={} travel={} rate=99.0", stamp(base + 180_000 + i * 1000), 1 + i % 3, if i == 0 { 12.345 } else { 20.0 + i as f64 * 0.5 })).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(20), || {
        let (_, t) = get(addr, "/s/loop/portal/api/tables");
        t["tables"].as_array().unwrap().iter().any(|x| x["name"] == "sorts" && x["rows"] == json!(53))
    }));
    let (_, before) = get(addr, "/s/loop/portal/api/tables");
    let mut page = Browser::watched(&format!("http://{addr}/s/loop/#tables"));

    // The Tables view lists the catalog as portal lists it: each button's name, kind and count.
    const LISTED: &str = r#"(() => { const b = [...document.querySelectorAll(".list button[data-table]")]; return b.length ? b.map((x) => [x.dataset.table, x.children[0].textContent, x.children[1].textContent]) : null; })()"#;
    let listed = page.wait_for(LISTED, Duration::from_secs(20));
    let (_, t) = get(addr, "/s/loop/portal/api/tables");
    let want: Vec<(String, String, String)> = t["tables"].as_array().unwrap().iter().map(|x| (x["name"].as_str().unwrap().to_string(), x["name"].as_str().unwrap().to_string(), x["kind"].as_str().unwrap().to_string())).collect();
    let got: Vec<(String, String, String)> = listed.as_array().unwrap().iter().map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_str().unwrap().to_string(), r[2].as_str().unwrap().split(" · ").next().unwrap().to_string())).collect();
    assert_eq!(got, want);
    assert!(listed.as_array().unwrap().iter().any(|r| r[0] == "sorts" && r[2] == json!(format!("{} · 53", sorts["kind"].as_str().unwrap()))), "{listed}");
    // Every count: a count portal does not know shown as "?" (the view's), the others between
    // what portal counted before the page listed them and after (the OPC rule's keeps growing).
    let rows_in = |t: &Value, name: &str| t["tables"].as_array().unwrap().iter().find(|x| x["name"] == name).unwrap()["rows"].clone();
    for r in listed.as_array().unwrap() {
        let name = r[0].as_str().unwrap();
        let (kind, count) = r[2].as_str().unwrap().split_once(" · ").unwrap();
        match (rows_in(&before, name).as_u64(), rows_in(&t, name).as_u64()) {
            (Some(lo), Some(hi)) => assert!(count.parse::<u64>().is_ok_and(|n| lo <= n && n <= hi), "{name}: {kind} · {count} outside {lo}..={hi}"),
            _ => assert_eq!((rows_in(&before, name), count), (Value::Null, "?"), "{name}: a count portal does not know"),
        }
    }
    assert!(listed.as_array().unwrap().iter().any(|r| r[0] == "outlets" && r[2] == "view · ?"), "{listed}");

    // A table picked: its first page of fifty, newest first, header (names and types) and every
    // cell as portal pages it, no newer page; the older page; back to the newer one.
    const SHOWN: &str = r#"(() => {
  const pager = document.querySelector(".list-main .pager [data-rows]");
  const button = (t) => [...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === t)?.disabled ?? null;
  return pager ? {
    rows: pager.dataset.rows, said: pager.textContent,
    head: [...document.querySelectorAll(".list-main thead th")].map((th) => th.textContent),
    body: [...document.querySelectorAll(".list-main tbody tr")].map((tr) => [...tr.children].map((td) => td.textContent)),
    newer: button("‹ Newer"), older: button("Older ›"),
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
            p["schema"].as_array().unwrap().iter().map(|c| format!("{} {}", c["name"].as_str().unwrap(), c["type"].as_str().unwrap().to_lowercase())).collect(),
            p["rows"].as_array().unwrap().iter().map(|r| r.as_array().unwrap().iter().map(cell).collect()).collect(),
        )
    };
    // A page is matched by its rows and by what it says of them, on every page: "rows A–B of N".
    let shown_at = |page: &mut Browser, rows: &str| {
        page.wait_for(
            &format!("(() => {{ const s = {SHOWN}; if (!s || s.rows !== '{rows}') return null; const [ab, n] = s.rows.split('/'); const [a, b] = ab.split('-'); return s.said === `rows ${{a}}–${{b}} of ${{n}}, newest first` ? s : null; }})()"),
            Duration::from_secs(20),
        )
    };
    let click = |page: &mut Browser, text: &str| {
        page.eval(&format!(r#"[...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "{text}").click()"#));
    };
    page.eval(r#"document.querySelector('.list button[data-table="sorts"]').click()"#);
    let first = shown_at(&mut page, "1-50/53");
    let (head, body) = as_shown(&get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=50&offset=0").1);
    assert_eq!((first["head"].clone(), first["body"].clone(), first["newer"].clone(), first["older"].clone()), (json!(head), json!(body), json!(true), json!(false)));
    assert_eq!(first["said"], json!("rows 1–50 of 53, newest first"));
    let travel = head.iter().position(|c| c.starts_with("travel ")).unwrap();
    assert_eq!((first["body"][0][travel].clone(), first["body"][49][travel].clone()), (json!("44.5"), json!("12.345")), "newest first, to three places");
    click(&mut page, "Older ›");
    let older = shown_at(&mut page, "51-53/53");
    let (_, body) = as_shown(&get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=50&offset=50").1);
    assert_eq!((older["body"].clone(), older["newer"].clone(), older["older"].clone()), (json!(body), json!(false), json!(true)));
    assert_eq!(older["body"].as_array().unwrap().iter().map(|r| r[travel].clone()).collect::<Vec<_>>(), [json!("11.5"), json!("12.5"), json!("10.5")]);
    click(&mut page, "‹ Newer");
    let back = shown_at(&mut page, "1-50/53");
    assert_eq!((back["body"].clone(), back["newer"].clone()), (first["body"].clone(), json!(true)), "back to the newest page whole");
    // Paged to its older page, then another table picked, then this one again: each opens on its
    // first page, whatever page the one before was on.
    let pick = |page: &mut Browser, name: &str| {
        page.eval(&format!(r#"document.querySelector('.list button[data-table="{name}"]').click()"#));
    };
    click(&mut page, "Older ›");
    shown_at(&mut page, "51-53/53");
    pick(&mut page, "plc_main");
    let other = page.wait_for(&format!("(() => {{ const s = {SHOWN}; return s && s.head.some((h) => h.startsWith('fill1 ')) ? s : null; }})()"), Duration::from_secs(20));
    // Its first page from row 1, whatever its count: the bench writes the table live, so a fast
    // host reaches here with fewer than fifty rows (CI showed "1-47/47").
    assert!(other["rows"].as_str().unwrap().starts_with("1-") && other["newer"] == json!(true), "{other}");
    pick(&mut page, "sorts");
    let again = shown_at(&mut page, "1-50/53");
    assert_eq!((again["body"].clone(), again["newer"].clone()), (first["body"].clone(), json!(true)), "the first page again");

    // The Series view lists the series as portal lists them: each button's names, type and count,
    // the count between portal's before the page listed and after (the OPC series keep counting;
    // the log's do not).
    let (_, before) = get(addr, "/s/loop/portal/api/series");
    page.eval(r#"location.hash = "series""#);
    let listed = page.wait_for(r#"(() => { const b = [...document.querySelectorAll(".list button[data-series]")]; return b.length ? b.map((x) => [x.dataset.series, x.children[0].textContent, x.children[1].textContent]) : null; })()"#, Duration::from_secs(20));
    let (_, s) = get(addr, "/s/loop/portal/api/series");
    let series = s["series"].as_array().unwrap();
    assert_eq!(listed.as_array().unwrap().len(), series.len());
    let count_before = |src: &str, ser: &str| before["series"].as_array().unwrap().iter().find(|y| y["source"] == json!(src) && y["series"] == json!(ser)).and_then(|y| y["count"].as_u64()).unwrap_or(0);
    for (b, x) in listed.as_array().unwrap().iter().zip(series) {
        let (src, ser, ty) = (x["source"].as_str().unwrap(), x["series"].as_str().unwrap(), x["type"].as_str().unwrap());
        assert_eq!((b[0].clone(), b[1].clone()), (json!(format!("{src}/{ser}")), json!(format!("{src} · {ser}"))));
        let (shown_ty, count) = b[2].as_str().unwrap().split_once(" · ").unwrap();
        let count: u64 = count.parse().unwrap();
        let (low, high) = (count_before(src, ser), x["count"].as_u64().unwrap());
        assert!(shown_ty == ty && low <= count && count <= high && (src != "sorter" || count == high), "{b} against {low}..={high}: {x}");
    }
    assert!(listed.as_array().unwrap().iter().any(|b| b[0] == "sorter/travel" && b[2] == "float · 53"), "{listed}");

    // A series picked: its last hour charted, every point of portal's window placed exactly where
    // the chart's axes put its time and value, the axis labelled by the window's values.
    page.eval(r#"document.querySelector('.list button[data-series="sorter/travel"]').click()"#);
    let chart = page.wait_for(
        r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const line = document.querySelector(".list-main svg.trend path.line"); return p && line ? { points: p.dataset.points, said: p.textContent, d: line.getAttribute("d"), axis: [...document.querySelectorAll(".list-main svg.trend text.axis")].map((t) => [t.textContent, Number(t.getAttribute("x")), Number(t.getAttribute("y"))]) } : null; })()"#,
        Duration::from_secs(20),
    );
    let last = series.iter().find(|x| x["source"] == "sorter" && x["series"] == "travel").unwrap()["last_value"].as_str().unwrap().to_string();
    assert_eq!((chart["points"].clone(), chart["said"].clone()), (json!("53/53"), json!(format!("last 60 min · 53 of 53 points · last {last}"))));
    // The window the page asked portal for: the last hour to the session's now, 600 points.
    let asked: Vec<String> = page.requests().into_iter().map(|(u, _)| u).filter(|u| u.contains("/portal/api/series/data?")).collect();
    assert_eq!(asked.len(), 1, "{asked:?}");
    let query = |k: &str| asked[0].split(['?', '&']).find_map(|kv| kv.strip_prefix(&format!("{k}="))).unwrap().to_string();
    let (from, to): (i64, i64) = (query("from").parse().unwrap(), query("to").parse().unwrap());
    assert_eq!((query("source"), query("series"), query("points"), to - from), ("sorter".to_string(), "travel".to_string(), "600".to_string(), 3_600_000));
    assert!((store_now() - to).abs() < 15_000, "the window ends at the session's now: {to}");
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
    let (_, w) = get(addr, &format!("/s/loop/portal/api/series/data?source=sorter&series=travel&from={from}&to={to}&points=600"));
    let points: Vec<(f64, f64)> = w["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
    assert_eq!(drawn.len(), points.len());
    // The trend's frame (charts.ts): 600 by 200, the plot from x 44 to 590 and from y 178 up to
    // 10, the values' extent padded by 8 % each way.
    let (mut lo, mut hi) = points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let pad = (hi - lo) * 0.08;
    (lo, hi) = (lo - pad, hi + pad);
    for (i, ((dx, dy), (t, v))) in drawn.iter().zip(&points).enumerate() {
        let x = 44.0 + (t - from as f64) / (to - from) as f64 * 546.0;
        let y = 178.0 + (v - lo) / (hi - lo) * (10.0 - 178.0);
        assert!((dx - x).abs() <= 0.051 && (dy - y).abs() <= 0.051, "point {i} ({t}, {v}) drawn at ({dx}, {dy}), its place ({x:.2}, {y:.2})");
    }
    let num1 = |v: f64| {
        let t = format!("{v:.1}");
        t.trim_end_matches('0').trim_end_matches('.').to_string()
    };
    // The value axis: four labels at a third apart, each beside its value's height (its baseline
    // 4 below it), from the bottom up.
    let near = |a: f64, b: f64| (a - b).abs() <= 0.051;
    let axis = chart["axis"].as_array().unwrap();
    assert_eq!(axis.len(), 9, "{axis:?}");
    for i in 0..4 {
        let v = lo + (hi - lo) * i as f64 / 3.0;
        let at = 178.0 + (v - lo) / (hi - lo) * (10.0 - 178.0) + 4.0;
        assert!(axis[i][0] == json!(num1(v)) && near(axis[i][1].as_f64().unwrap(), 38.0) && near(axis[i][2].as_f64().unwrap(), at), "value label {i}: {} against {} at (38, {at:.2})", axis[i], num1(v));
    }
    // The time axis: five labels a quarter of the window apart, each the store's clock (hours and
    // minutes) at its place, the last "now".
    for i in 0..5 {
        let t = from as f64 + (to - from) as f64 * i as f64 / 4.0;
        let said = if i == 4 { "now".to_string() } else { chrono::DateTime::from_timestamp_millis(t as i64).unwrap().format("%H:%M").to_string() };
        let x = 44.0 + (t - from as f64) / (to - from) as f64 * 546.0;
        assert!(axis[4 + i][0] == json!(said) && near(axis[4 + i][1].as_f64().unwrap(), x) && near(axis[4 + i][2].as_f64().unwrap(), 194.0), "time label {i}: {} against {said} at ({x:.2}, 194)", axis[4 + i]);
    }

    // A table of exactly a hundred rows: the second page is the last, nothing older.
    append(&log, &(0..47).map(|i| format!("{} SORT outlet=1 travel=30.0 rate=99.0", stamp(base + 240_000 + i * 1000))).collect::<Vec<_>>());
    assert!(wait(Duration::from_secs(20), || {
        let (_, t) = get(addr, "/s/loop/portal/api/tables");
        t["tables"].as_array().unwrap().iter().any(|x| x["name"] == "sorts" && x["rows"] == json!(100))
    }));
    page.eval(r#"location.hash = "tables""#);
    shown_at(&mut page, "1-50/100");
    click(&mut page, "Older ›");
    let last_page = shown_at(&mut page, "51-100/100");
    assert_eq!((last_page["body"].as_array().unwrap().len(), last_page["older"].clone()), (50, json!(true)));

    // Every request the page made went to this host and this session's own paths, its portal
    // reads the four routes under `/s/loop/portal/api/`, its stream the session's objects.
    let requests = page.requests();
    let origin = format!("http://{addr}");
    let paths: Vec<(String, String)> = requests
        .iter()
        .map(|(u, ty)| {
            assert!(u.starts_with(&format!("{origin}/")), "a request off the host: {u}");
            (u[origin.len()..].split('?').next().unwrap().to_string(), ty.clone())
        })
        .collect();
    for route in ["/s/loop/portal/api/tables", "/s/loop/portal/api/table/data", "/s/loop/portal/api/series", "/s/loop/portal/api/series/data"] {
        assert!(paths.iter().any(|(p, _)| p == route), "{route} in {paths:?}");
    }
    assert!(paths.iter().any(|(p, ty)| p == "/s/loop/objects" && ty == "EventSource"), "streams are seen: {paths:?}");
    assert!(paths.iter().all(|(_, ty)| ty != "WebSocket"), "no WebSocket: {paths:?}");
    // (The browser asks for `/favicon.ico` on its own.)
    assert!(paths.iter().all(|(p, _)| p.starts_with("/assets/") || p.starts_with("/s/loop/") || p == "/api/sessions" || p == "/favicon.ico"), "{paths:?}");
    assert!(
        paths.iter().filter(|(p, _)| p.contains("/portal")).all(|(p, _)| ["/s/loop/portal/api/tables", "/s/loop/portal/api/table/data", "/s/loop/portal/api/series", "/s/loop/portal/api/series/data"].contains(&p.as_str())),
        "{paths:?}"
    );
    drop(page);

    // The other session's page reads the other session's router alone: its catalog (no sort
    // logged there), never this one's.
    let mut page = Browser::watched(&format!("http://{addr}/s/hall/#tables"));
    let listed = page.wait_for(LISTED, Duration::from_secs(20));
    let (_, t) = get(addr, "/s/hall/portal/api/tables");
    let want: Vec<(Value, Value)> = t["tables"].as_array().unwrap().iter().map(|x| (x["name"].clone(), json!(format!("{} · {}", x["kind"].as_str().unwrap(), x["rows"])))).collect();
    let sorts_there = want.iter().find(|(n, _)| n == "sorts").unwrap().1.clone();
    assert_eq!(sorts_there, json!(format!("{} · 0", sorts["kind"].as_str().unwrap())), "hall logged no sort");
    assert!(listed.as_array().unwrap().iter().any(|r| r[0] == "sorts" && r[2] == sorts_there), "{listed}");
    // Its Series view lists hall's series alone; one sort logged there makes a window of one point,
    // drawn as a dot where the axes put it (the value in the middle of its padded extent).
    let at = store_now() - 5 * 60_000;
    append(&hall_log, &[format!("{} SORT outlet=1 travel=10.5 rate=99.0", stamp(at))]);
    assert!(wait(Duration::from_secs(20), || {
        let (_, s) = get(addr, "/s/hall/portal/api/series");
        s["series"].as_array().unwrap().iter().any(|x| x["source"] == "sorter" && x["series"] == "travel" && x["count"] == json!(1))
    }));
    page.eval(r#"location.hash = "series""#);
    let listed = page.wait_for(r#"(() => { const b = [...document.querySelectorAll(".list button[data-series]")]; return b.some((x) => x.dataset.series === "sorter/travel") ? b.map((x) => x.dataset.series) : null; })()"#, Duration::from_secs(20));
    let (_, s) = get(addr, "/s/hall/portal/api/series");
    let names: Vec<String> = s["series"].as_array().unwrap().iter().map(|x| format!("{}/{}", x["source"].as_str().unwrap(), x["series"].as_str().unwrap())).collect();
    assert_eq!(listed, json!(names), "hall's series, as hall's portal lists them");
    page.eval(r#"document.querySelector('.list button[data-series="sorter/travel"]').click()"#);
    let one = page.wait_for(
        r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const dot = document.querySelector(".list-main svg.trend circle.point"); return p && dot ? { points: p.dataset.points, said: p.textContent, line: document.querySelectorAll(".list-main svg.trend path.line").length, cx: Number(dot.getAttribute("cx")), cy: Number(dot.getAttribute("cy")), shown: (() => { const c = getComputedStyle(dot), b = dot.getBoundingClientRect(); const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true }); ctx.fillStyle = "rgba(0, 0, 0, 0)"; ctx.fillStyle = c.fill; ctx.clearRect(0, 0, 1, 1); ctx.fillRect(0, 0, 1, 1); const alpha = c.fill === "none" ? 0 : ctx.getImageData(0, 0, 1, 1).data[3] / 255; let o = 1; for (let e = dot; e; e = e.parentElement) { const s = getComputedStyle(e); o *= Number(s.opacity); if (s.display === "none") o = 0; } return b.width > 0 && b.height > 0 && c.visibility === "visible" && o > 0 && alpha * Number(c.fillOpacity) > 0 && document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2) === dot; })() } : null; })()"#,
        Duration::from_secs(20),
    );
    let asked: Vec<String> = page.requests().into_iter().map(|(u, _)| u).filter(|u| u.contains("/portal/api/series/data?")).collect();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].contains("/s/hall/portal/api/series/data?"), "{asked:?}");
    let query = |k: &str| asked[0].split(['?', '&']).find_map(|kv| kv.strip_prefix(&format!("{k}="))).unwrap().to_string();
    let (from, to): (i64, i64) = (query("from").parse().unwrap(), query("to").parse().unwrap());
    let x = 44.0 + (at - from) as f64 / (to - from) as f64 * 546.0;
    assert_eq!((one["points"].clone(), one["line"].clone(), one["shown"].clone()), (json!("1/1"), json!(0), json!(true)), "{one}");
    assert!((one["cx"].as_f64().unwrap() - x).abs() <= 0.051 && (one["cy"].as_f64().unwrap() - 94.0).abs() <= 0.051, "the one point drawn at ({}, {}), its place ({x:.2}, 94)", one["cx"], one["cy"]);
    let requests = page.requests();
    assert!(requests.iter().all(|(u, ty)| u.starts_with(&format!("http://{addr}/")) && ty != "WebSocket"), "{requests:?}");
    let paths: Vec<String> = requests.into_iter().map(|(u, _)| u).collect();
    assert!(paths.iter().any(|u| u.contains("/s/hall/portal/api/tables")) && paths.iter().any(|u| u.contains("/s/hall/portal/api/series?") || u.ends_with("/s/hall/portal/api/series")), "{paths:?}");
    assert!(paths.iter().all(|u| !u.contains("/s/loop/")), "{paths:?}");
    assert!(paths.iter().filter(|u| u.contains("/portal")).all(|u| u.contains("/s/hall/portal/api/")), "{paths:?}");
    drop(page);
    host.stop();
}
