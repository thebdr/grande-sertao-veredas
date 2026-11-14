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

/// A view with no time column (portal keeps no order for it) and a boolean column.
const VIEW: &str = "slows";

/// Shown, as a person would see it: a box, visible, displayed through every ancestor, and painted
/// with some alpha once every opacity is multiplied with its paint's (an SVG shape's fill or stroke
/// and their opacities, a text's colour). A mask, a filter, a font or a cover that takes no pointer
/// is not read (the clause states the limit).
const SHOWN_FN: &str = r#"const shown = (el) => {
  if (!el) return false;
  const b = el.getBoundingClientRect();
  if (!(b.width > 0 || b.height > 0)) return false;
  const c = getComputedStyle(el);
  if (c.visibility !== "visible") return false;
  let o = 1;
  for (let e = el; e; e = e.parentElement) { const s = getComputedStyle(e); if (s.display === "none") return false; o *= Number(s.opacity); }
  const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true });
  const alpha = (p) => { if (!p || p === "none") return 0; if (p.startsWith("url(")) return 1; ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = "rgba(0, 0, 0, 0)"; ctx.fillStyle = p; ctx.fillRect(0, 0, 1, 1); return ctx.getImageData(0, 0, 1, 1).data[3] / 255; };
  const paint = el instanceof SVGElement ? Math.max(alpha(c.fill) * Number(c.fillOpacity), parseFloat(c.strokeWidth) > 0 ? alpha(c.stroke) * Number(c.strokeOpacity) : 0) : alpha(c.color);
  return o * paint > 0.05;
};"#;

/// Every mark of the view shown (`SHOWN_FN`'s reading): every element under the view with text of
/// its own and every SVG shape, the trend's hidden hover tip apart; the ones not shown, named.
const EVERY_MARK: &str = r#"[...document.querySelector("main.view").querySelectorAll("*")]
  .filter((el) => !el.closest(".tip, defs, title") && ([...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()) || el.matches("svg path, svg circle, svg rect, svg line, svg polyline, svg polygon, svg ellipse")))
  .filter((el) => !shown(el))
  .map((el) => `${el.tagName}.${el.getAttribute("class")} ${JSON.stringify((el.textContent || "").slice(0, 40))}`)"#;

/// Every place of a view whose words this test compares with portal's answer or with what it
/// asked (the lists' captions and buttons, a page's heading, its count and its buttons, a table's
/// header and cells, a window's count and its axis, a refusal and the words before a pick).
const READ_MARKS: &str = r#"[".list > .caps", ".list button > span", ".list-main > .pager > strong", ".list-main > .pager > [data-rows]", ".list-main > .pager > [data-points]", ".list-main > .pager > button", ".list-main thead th", ".list-main thead th > span", ".list-main tbody td", ".list-main svg.trend text.axis", ".list-main > p.error", ".list-main > p.muted"]"#;

fn every_mark_shown(page: &mut Browser, when: &str) {
    let hidden = page.eval(&format!("(() => {{ {SHOWN_FN} return {EVERY_MARK}; }})()"));
    assert_eq!(hidden, json!([]), "{when}: a mark of the view not shown");
    // And every mark with words of its own is one whose words this test reads: a mark added to a
    // view fails here until it is.
    let unread = page.eval(&format!(
        r#"(() => {{ const READ = {READ_MARKS}; return [...document.querySelector("main.view").querySelectorAll("*")].filter((el) => !el.closest(".tip, defs, title") && [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim()) && !READ.some((s) => el.matches(s))).map((el) => `${{el.tagName}}.${{el.getAttribute("class")}} ${{JSON.stringify(el.textContent.slice(0, 40))}}`); }})()"#
    ));
    assert_eq!(unread, json!([]), "{when}: a mark whose words this test does not read");
}

/// Kept in the page: every answer portal gave the page's own fetches of rows and windows, by its
/// address; while `__fail` is set, the next page of rows refused as portal refuses one; and the
/// next request whose address holds `__slow` answered a second and a half late (refused then, as
/// portal refuses, while `__slowRefuse` is set), so an answer comes after a later request's;
/// `__slowDone` set once the page has had it.
const KEEP_ANSWERS: &str = r#"(() => {
  window.__answers = [];
  const orig = window.fetch;
  window.fetch = async (...a) => {
    const url = String(a[0]);
    if (window.__fail && url.includes("/api/table/data")) {
      window.__fail = false;
      return new Response(JSON.stringify({ error: "probe failure" }), { status: 500, headers: { "content-type": "application/json" } });
    }
    const slow = window.__slow && url.includes(window.__slow);
    const refuse = slow && window.__slowRefuse;
    if (slow) { window.__slow = null; window.__slowRefuse = false; window.__slowDone = false; window.__slowSent = true; }
    const r = await orig(...a);
    if (slow) { await new Promise((done) => setTimeout(done, 1500)); setTimeout(() => { window.__slowDone = true; }, 300); }
    if (refuse) return new Response(JSON.stringify({ error: "probe refusal, late" }), { status: 500, headers: { "content-type": "application/json" } });
    if (!url.includes("/api/table/data") && !url.includes("/api/series/data")) return r;
    const j = await r.clone().json();
    window.__answers.push({ url, body: j });
    return r;
  };
  return true;
})()"#;

/// A query's value as a server reads it: `+` a space, `%XX` its byte.
fn unescape(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                out.push(u8::from_str_radix(&v[i + 1..i + 3], 16).unwrap());
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

/// A request's query value by its name, unescaped.
fn param(url: &str, k: &str) -> Option<String> {
    url.split_once('?')?.1.split('&').find_map(|kv| kv.strip_prefix(&format!("{k}="))).map(unescape)
}

/// Where a trend's line differs from the window it draws (`charts.ts`: 600 by 200, the plot from x
/// 44 to 590 and from y 178 up to 10, the values' extent padded by 8 % each way), point by point.
fn misplaced(d: &str, points: &[(f64, f64)], from: i64, to: i64) -> Vec<String> {
    let drawn: Vec<(f64, f64)> = d
        .split(['M', 'L'])
        .filter(|p| !p.trim().is_empty())
        .map(|p| {
            let mut it = p.split_whitespace().map(|n| n.parse::<f64>().unwrap());
            (it.next().unwrap(), it.next().unwrap())
        })
        .collect();
    if drawn.len() != points.len() {
        return vec![format!("{} points drawn for {}", drawn.len(), points.len())];
    }
    let (mut lo, mut hi) = points.iter().fold((f64::MAX, f64::MIN), |(a, b), p| (a.min(p.1), b.max(p.1)));
    let pad = (hi - lo) * 0.08;
    (lo, hi) = (lo - pad, hi + pad);
    drawn
        .iter()
        .zip(points)
        .enumerate()
        .filter_map(|(i, ((dx, dy), (t, v)))| {
            let x = 44.0 + (t - from as f64) / (to - from) as f64 * 546.0;
            let y = if hi > lo { 178.0 + (v - lo) / (hi - lo) * (10.0 - 178.0) } else { 94.0 };
            ((dx - x).abs() > 0.051 || (dy - y).abs() > 0.051).then(|| format!("point {i} ({t}, {v}) drawn at ({dx}, {dy}), its place ({x:.2}, {y:.2})"))
        })
        .collect()
}

#[test]
fn a_sessions_tables_rows_series_and_windows_read_through_its_portal() {
    // Two plants on one host, so the page's reading of its own session is told from another's.
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(&dir.path().join("bench"));
    let other = plant(&dir.path().join("bench-hall"));
    let path = two_plants(dir.path(), &bench, &other);
    // A view of the sorts with a boolean column and no time column, so portal keeps no order for
    // it. (Portal names a table or a series by lowercase letters, digits and `_` alone, so no name
    // the page sends in a query needs escaping.)
    let project = dir.path().join("loop/project.toml");
    let text = std::fs::read_to_string(&project).unwrap();
    // And a view with `ts` alone for its time, so every way portal finds a table's time (its own
    // rule: `index_ms`, else `ts`, else none) is a table here; and one of numbers from 1e21 up,
    // where a script writes a number with an exponent (and one just below), each exact in binary,
    // and numbers below 0 a script writes as it is (a fraction, a whole one, one that rounds to 0).
    std::fs::write(&project, format!("{text}\n[[table]]\nname = \"{VIEW}\"\nsql = \"SELECT outlet, travel > 12 AS slow FROM sorts\"\n\n[[table]]\nname = \"stamped\"\nsql = \"SELECT ts, outlet FROM sorts\"\n\n[[table]]\nname = \"magnitudes\"\nsql = \"SELECT 9.5e20::DOUBLE AS below, 1e21::DOUBLE AS edge, -3.5e21::DOUBLE AS negative, 1e22::DOUBLE AS big, -0.25::DOUBLE AS quarter, -7.0::DOUBLE AS seven, -0.0001::DOUBLE AS tiny\"\n")).unwrap();
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
    assert_eq!(names.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["magnitudes", "outlets", "plc_main", "samples", VIEW, "sorts", "stamped"]);
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
    page.eval(KEEP_ANSWERS);

    // The Tables view lists the catalog as portal lists it: each button's name, kind and count.
    const LISTED: &str = r#"(() => { const b = [...document.querySelectorAll(".list button[data-table]")]; return b.length ? b.map((x) => [x.dataset.table, x.children[0].textContent, x.children[1].textContent]) : null; })()"#;
    let listed = page.wait_for(LISTED, Duration::from_secs(20));
    let (_, t) = get(addr, "/s/loop/portal/api/tables");
    let want: Vec<(String, String, String)> = t["tables"].as_array().unwrap().iter().map(|x| (x["name"].as_str().unwrap().to_string(), x["name"].as_str().unwrap().to_string(), x["kind"].as_str().unwrap().to_string())).collect();
    let got: Vec<(String, String, String)> = listed.as_array().unwrap().iter().map(|r| (r[0].as_str().unwrap().to_string(), r[1].as_str().unwrap().to_string(), r[2].as_str().unwrap().split(" · ").next().unwrap().to_string())).collect();
    assert_eq!(got, want);
    assert!(listed.as_array().unwrap().iter().any(|r| r[0] == "sorts" && r[2] == json!(format!("{} · 53", sorts["kind"].as_str().unwrap()))), "{listed}");
    // Its caption the catalog's length; before a pick, the words asking for one.
    assert_eq!(page.eval(r#"[document.querySelector(".list > .caps")?.textContent ?? null, document.querySelector(".list-main > p.muted")?.textContent ?? null]"#), json!([format!("Tables · {}", t["tables"].as_array().unwrap().len()), "Pick a table."]));
    every_mark_shown(&mut page, "the catalog before a pick");
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
    let shown_js = format!(
        r#"(() => {{
  {SHOWN_FN}
  const pager = document.querySelector(".list-main .pager [data-rows]");
  const button = (t) => [...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === t)?.disabled ?? null;
  return pager ? {{
    table: document.querySelector(".list-main .pager strong").textContent,
    all_shown: [pager, ...document.querySelectorAll(".list-main thead th, .list-main tbody td")].every(shown),
    rows: pager.dataset.rows, said: pager.textContent,
    head: [...document.querySelectorAll(".list-main thead th")].map((th) => th.textContent),
    body: [...document.querySelectorAll(".list-main tbody tr")].map((tr) => [...tr.children].map((td) => td.textContent)),
    newer: button("‹ Newer"), older: button("Older ›"),
  }} : null;
}})()"#
    );
    let shown_js = shown_js.as_str();
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
    // A page of the sorts is matched by its table, its rows and what it says of them ("rows A–B of
    // N"), everything on it shown, and its buttons by its place (Newer off on the first page alone,
    // Older off on the last alone).
    let shown_at = |page: &mut Browser, rows: &str| {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let (ab, n) = rows.split_once('/').unwrap();
        let (a, b) = ab.split_once('-').unwrap();
        let said = format!("rows {a}–{b} of {n}, newest first");
        loop {
            let s = page.eval(shown_js);
            if s["table"] == json!("sorts") && s["all_shown"] == json!(true) && s["rows"] == json!(rows) && s["said"] == json!(said) && s["newer"] == json!(a == "1") && s["older"] == json!(b == n) {
                return s;
            }
            assert!(std::time::Instant::now() < deadline, "the sorts never shown at {rows}; the page shows {}", json!({ "table": s["table"], "rows": s["rows"], "said": s["said"], "newer": s["newer"], "older": s["older"], "all_shown": s["all_shown"] }));
            std::thread::sleep(Duration::from_millis(150));
        }
    };
    let click = |page: &mut Browser, text: &str| {
        page.eval(&format!(r#"[...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "{text}").click()"#));
    };
    page.wait_for(r#"/\d\d:\d\d:\d\d/.test(document.querySelector(".live")?.textContent ?? "") || null"#, Duration::from_secs(20));
    page.eval(r#"document.querySelector('.list button[data-table="sorts"]').click()"#);
    let first = shown_at(&mut page, "1-50/53");
    let (head, body) = as_shown(&get(addr, "/s/loop/portal/api/table/data?name=sorts&limit=50&offset=0").1);
    assert_eq!((first["head"].clone(), first["body"].clone(), first["newer"].clone(), first["older"].clone()), (json!(head), json!(body), json!(true), json!(false)));
    assert_eq!(first["said"], json!("rows 1–50 of 53, newest first"));
    every_mark_shown(&mut page, "a table's first page");
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
    let other = page.wait_for(&format!("(() => {{ const s = {shown_js}; return s && s.head.some((h) => h.startsWith('fill1 ')) ? s : null; }})()"), Duration::from_secs(20));
    // Its first page from row 1, whatever its count: the bench writes the table live, so a fast
    // host reaches here with fewer than fifty rows (CI showed "1-47/47").
    assert!(other["rows"].as_str().unwrap().starts_with("1-") && other["newer"] == json!(true), "{other}");
    pick(&mut page, "sorts");
    let again = shown_at(&mut page, "1-50/53");
    assert_eq!((again["body"].clone(), again["newer"].clone()), (first["body"].clone(), json!(true)), "the first page again");
    // Every table portal's catalog lists, opened in turn: its first page as portal answered the
    // page's own request (the table by its name however it must be escaped, fifty rows from the
    // first, the window frozen at the session's now), every header and cell as that answer has
    // them, "newest first" said of a table portal orders by time alone, all of it shown. Across
    // them, every kind of value portal's rows carry: a null, a boolean, a number and a text.
    let (_, catalog) = get(addr, "/s/loop/portal/api/tables");
    let mut kinds = std::collections::BTreeSet::new();
    let mut times = std::collections::BTreeSet::new();
    let mut paged = std::collections::BTreeSet::new();
    for x in catalog["tables"].as_array().unwrap() {
        let name = x["name"].as_str().unwrap();
        let quoted = serde_json::to_string(name).unwrap();
        page.eval("window.__answers = []; true");
        page.eval(&format!("[...document.querySelectorAll('.list button[data-table]')].find((b) => b.dataset.table === {quoted}).click()"));
        let s = page.wait_for(&format!("(() => {{ const s = {shown_js}; return s && s.table === {quoted} && window.__answers.length > 0 ? s : null; }})()"), Duration::from_secs(20));
        let answers = page.eval("window.__answers");
        let last = answers.as_array().unwrap().last().unwrap().clone();
        let url = last["url"].as_str().unwrap();
        assert_eq!((param(url, "name").as_deref(), param(url, "limit").as_deref(), param(url, "offset").as_deref()), (Some(name), Some("50"), Some("0")), "{url}");
        assert!(param(url, "to").is_some_and(|t| (store_now() - t.parse::<i64>().unwrap()).abs() < 60_000), "the window frozen at the session's now: {url}");
        let p = &last["body"];
        let (head, body) = as_shown(p);
        let n = p["rows"].as_array().unwrap().len();
        let total = p["total"].as_u64().unwrap();
        let has = |c: &str| p["schema"].as_array().unwrap().iter().any(|x| x["name"] == c);
        let time_kind = if has("index_ms") { "index_ms" } else if has("ts") { "ts" } else { "none" };
        times.insert(time_kind);
        let ordered = has("index_ms") || has("ts");
        let said = format!("rows {}–{n} of {total}{}", if total == 0 { 0 } else { 1 }, if ordered { ", newest first" } else { ", unordered" });
        assert_eq!((s["head"].clone(), s["body"].clone(), s["said"].clone(), s["all_shown"].clone()), (json!(head), json!(body), json!(said), json!(true)), "{name}");
        // Its first page's buttons by its place: Newer off, Older off when it is the last too.
        assert_eq!((s["newer"].clone(), s["older"].clone()), (json!(true), json!(total <= 50)), "{name}: its pager on a first page of {total}");
        // A table of more than one page, paged to its second page as portal answered the page's own
        // request and back to its first whole: every way portal finds a table's time among them.
        if total > 50 {
            page.eval("window.__answers = []; true");
            click(&mut page, "Older ›");
            let second = page.wait_for(&format!("(() => {{ const s = {shown_js}; return s && s.table === {quoted} && s.rows.startsWith('51-') && window.__answers.length > 0 ? s : null; }})()"), Duration::from_secs(20));
            let answers = page.eval("window.__answers");
            let answer = answers.as_array().unwrap().last().unwrap();
            assert_eq!((param(answer["url"].as_str().unwrap(), "offset").as_deref(), param(answer["url"].as_str().unwrap(), "to")), (Some("50"), param(url, "to")), "{name}: the second page of the same window");
            let (h2, b2) = as_shown(&answer["body"]);
            assert_eq!((second["head"].clone(), second["body"].clone(), second["newer"].clone()), (json!(h2), json!(b2), json!(false)), "{name}: its second page");
            click(&mut page, "‹ Newer");
            let back = page.wait_for(&format!("(() => {{ const s = {shown_js}; return s && s.table === {quoted} && s.rows.startsWith('1-') ? s : null; }})()"), Duration::from_secs(20));
            assert_eq!(back["body"], s["body"], "{name}: back to its first page whole");
            paged.insert(time_kind);
        }
        for r in p["rows"].as_array().unwrap() {
            for v in r.as_array().unwrap() {
                kinds.insert(match v {
                    Value::Null => "null",
                    Value::Bool(_) => "boolean",
                    Value::Number(_) => "number",
                    Value::String(_) => "text",
                    _ => "other",
                });
            }
        }
    }
    assert_eq!(kinds.into_iter().collect::<Vec<_>>(), ["boolean", "null", "number", "text"], "every kind of value portal's rows carry");
    // And every way portal finds a table's time, by its own rule (`index_ms`, else `ts`, else
    // none): each a table here, said newest first or unordered as portal orders it.
    assert_eq!(times.into_iter().collect::<Vec<_>>(), ["index_ms", "none", "ts"], "every way portal finds a table's time");
    assert_eq!(paged.into_iter().collect::<Vec<_>>(), ["index_ms", "none", "ts"], "a table of every way paged and back");

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
    // Its caption the listing's length; before a pick, the words asking for one.
    assert_eq!(page.eval(r#"[document.querySelector(".list > .caps")?.textContent ?? null, document.querySelector(".list-main > p.muted")?.textContent ?? null]"#), json!([format!("Series · {}", listed.as_array().unwrap().len()), "Pick a series."]));
    every_mark_shown(&mut page, "the series before a pick");

    // A series picked: its last hour charted, every point of portal's window placed exactly where
    // the chart's axes put its time and value, the axis labelled by the window's values.
    page.eval(r#"document.querySelector('.list button[data-series="sorter/travel"]').click()"#);
    let chart = page.wait_for(
        &format!(
            "(() => {{ {SHOWN_FN} const r = {}; if (r) {{ r.line_shown = shown(document.querySelector('.list-main svg.trend path.line')); r.axis_shown = [...document.querySelectorAll('.list-main svg.trend text.axis')].every(shown); }} return r; }})()",
            r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const line = document.querySelector(".list-main svg.trend path.line"); return p && line ? { points: p.dataset.points, said: p.textContent, d: line.getAttribute("d"), axis: [...document.querySelectorAll(".list-main svg.trend text.axis")].map((t) => [t.textContent, Number(t.getAttribute("x")), Number(t.getAttribute("y"))]) } : null; })()"#
        ),
        Duration::from_secs(20),
    );
    assert_eq!((chart["line_shown"].clone(), chart["axis_shown"].clone()), (json!(true), json!(true)), "the line and every label shown");
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

    // Every shape of window portal answers: one point (hall's, below), many (the travel's, above),
    // and past portal's 600 (the speed, polled every 50 ms, once it holds more than 650 samples):
    // each drawn as the page's own answer has it, its words, every point placed, the line shown.
    let chart_of = |page: &mut Browser, key: &str| -> Value {
        let quoted = serde_json::to_string(key).unwrap();
        let head = serde_json::to_string(&key.replacen('/', " · ", 1)).unwrap();
        page.eval("window.__answers = []; true");
        page.eval(&format!("[...document.querySelectorAll('.list button[data-series]')].find((b) => b.dataset.series === {quoted}).click()"));
        page.wait_for(
            &format!("(() => {{ {SHOWN_FN} const p = document.querySelector('.list-main .pager [data-points]'); const line = document.querySelector('.list-main svg.trend path.line'); const head = document.querySelector('.list-main .pager strong'); const a = window.__answers.filter((x) => x.url.includes('/api/series/data')); return p && line && head && head.textContent === {head} && a.length ? {{ said: p.textContent, d: line.getAttribute('d'), line_shown: shown(line), axis_shown: [...document.querySelectorAll('.list-main svg.trend text.axis')].every(shown), answer: a[a.length - 1] }} : null; }})()"),
            Duration::from_secs(20),
        )
    };
    let drawn_as_answered = |chart: &Value, source: &str, series: &str| -> (u64, bool) {
        let url = chart["answer"]["url"].as_str().unwrap();
        assert_eq!((param(url, "source").as_deref(), param(url, "series").as_deref(), param(url, "points").as_deref()), (Some(source), Some(series), Some("600")), "{url}");
        let (from, to): (i64, i64) = (param(url, "from").unwrap().parse().unwrap(), param(url, "to").unwrap().parse().unwrap());
        assert_eq!(to - from, 3_600_000);
        let body = &chart["answer"]["body"];
        let points: Vec<(f64, f64)> = body["points"].as_array().unwrap().iter().map(|p| (p[0].as_f64().unwrap(), p[1].as_f64().unwrap())).collect();
        let (total, down) = (body["total"].as_u64().unwrap(), body["downsampled"] == json!(true));
        let said = format!("last 60 min · {} of {total} points{} · last ", points.len(), if down { ", downsampled" } else { "" });
        assert!(chart["said"].as_str().unwrap().starts_with(&said), "{} against {said}", chart["said"]);
        assert_eq!(misplaced(chart["d"].as_str().unwrap(), &points, from, to), Vec::<String>::new(), "{source}/{series}");
        assert_eq!((chart["line_shown"].clone(), chart["axis_shown"].clone()), (json!(true), json!(true)), "{source}/{series}");
        (total, down)
    };
    let speed_count = |s: &Value| s["series"].as_array().unwrap().iter().find(|x| x["source"] == "plc" && x["series"] == "speed").and_then(|x| x["count"].as_u64()).unwrap_or(0);
    assert!(wait(Duration::from_secs(120), || speed_count(&get(addr, "/s/loop/portal/api/series").1) > 650), "the speed polled past 650 samples");
    page.eval(r#"location.hash = "tables""#);
    page.eval(r#"location.hash = "series""#);
    page.wait_for(r#"[...document.querySelectorAll(".list button[data-series]")].some((b) => b.dataset.series === "plc/speed") || null"#, Duration::from_secs(20));
    let chart = chart_of(&mut page, "plc/speed");
    let (total, down) = drawn_as_answered(&chart, "plc", "speed");
    every_mark_shown(&mut page, "a chart past 600 points");
    // Its total no more than portal's for the same window asked again (the listing's count lags
    // the window's).
    let url = chart["answer"]["url"].as_str().unwrap();
    let (_, again) = get(addr, &format!("/s/loop/portal/api/series/data?source=plc&series=speed&from={}&to={}&points=600", param(url, "from").unwrap(), param(url, "to").unwrap()));
    assert!(down && total > 600 && total <= again["total"].as_u64().unwrap(), "600 of {total} points, downsampled, against {}", again["total"]);
    // Two series picked one after the other, the first one's window asked and answered last: the
    // later is the one charted.
    let series_button = |k: &str| format!(r#"[...document.querySelectorAll(".list button[data-series]")].find((x) => x.dataset.series === "{k}").click()"#);
    page.eval(&format!(r#"(() => {{ window.__slowSent = false; window.__slow = "series=travel&"; {}; return true; }})()"#, series_button("sorter/travel")));
    page.wait_for("window.__slowSent || null", Duration::from_secs(10));
    page.eval(&series_button("plc/speed"));
    const HEAD: &str = r#"document.querySelector(".list-main .pager strong")?.textContent ?? null"#;
    page.wait_for(&format!("({HEAD}) === 'plc · speed' || null"), Duration::from_secs(20));
    page.wait_for("window.__slowDone || null", Duration::from_secs(10));
    assert_eq!(page.eval(HEAD), json!("plc · speed"), "the later pick stays, the earlier window come late");
    // A series picked after its value moved since the listing shown was drawn: its heading says
    // the last value of the listing its own click fetched.
    bench.server("plc").unwrap().set("fill1", 65.0);
    let fill1_last = || get(addr, "/s/loop/portal/api/series").1["series"].as_array().unwrap().iter().find(|x| x["source"] == "plc" && x["series"] == "fill1").map(|x| x["last_value"].clone());
    assert!(wait(Duration::from_secs(20), || fill1_last() == Some(json!("65"))), "fill1 listed at 65: {:?}", fill1_last());
    assert!(page.eval(r#"[...document.querySelectorAll(".list button[data-series]")].some((b) => b.dataset.series === "plc/fill1")"#) == json!(true));
    page.eval(&series_button("plc/fill1"));
    let heading = page.wait_for(r#"(() => { const s = document.querySelector(".list-main .pager strong"), p = document.querySelector(".list-main .pager [data-points]"); return s && p && s.textContent === "plc · fill1" ? p.textContent : null; })()"#, Duration::from_secs(20));
    assert!(heading.as_str().unwrap().ends_with(" · last 65"), "{heading}");

    // A table of exactly a hundred rows: the second page is the last, nothing older.
    append(&log, &(0..47).map(|i| format!("{} SORT outlet=1 travel=30.0 rate=99.0", stamp(base + 240_000 + i * 1000))).collect::<Vec<_>>());
    let sorts_rows = |n: u64| {
        wait(Duration::from_secs(20), || {
            let (_, t) = get(addr, "/s/loop/portal/api/tables");
            t["tables"].as_array().unwrap().iter().any(|x| x["name"] == "sorts" && x["rows"] == json!(n))
        })
    };
    assert!(sorts_rows(100));
    page.eval(r#"location.hash = "tables""#);
    pick(&mut page, "sorts");
    let newest = shown_at(&mut page, "1-50/100");
    // Ten sorts stored now, while the first page is shown: the window the view froze when it was
    // shown pages on unshifted, no row of the first page shown again.
    let now = store_now();
    append(&log, &(0..10).map(|i| format!("{} SORT outlet=9 travel=1.0 rate=99.0", stamp(now + i))).collect::<Vec<_>>());
    assert!(sorts_rows(110));
    // "Older ›" clicked twice at once: one older page, never one past the end.
    page.eval(r#"(() => { const b = [...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "Older ›"); b.click(); b.click(); return true; })()"#);
    let last_page = shown_at(&mut page, "51-100/100");
    assert_eq!((last_page["body"].as_array().unwrap().len(), last_page["older"].clone()), (50, json!(true)));
    assert!(last_page["body"].as_array().unwrap().iter().all(|r| !newest["body"].as_array().unwrap().contains(r)), "a row of the first page shown again");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(page.eval(shown_js)["rows"], json!("51-100/100"), "still the last page, nothing past it");
    // The view shown anew without a pick (the series view, then this one: the page's one route for
    // a hash change, a tab or back): the window taken again, the rows stored since in it, the page
    // the same, now a middle one, both its neighbours live. The session's now is the one of the
    // status the page last received (a status a second): shown once the page holds one from past
    // the ten sorts (its live clock a second past their stamps), so its window takes them all.
    let past = chrono::DateTime::from_timestamp_millis(now + 1_009).unwrap().format("%H:%M:%S").to_string();
    page.wait_for(&format!(r#"(() => {{ const m = (document.querySelector(".live")?.textContent ?? "").match(/(\d\d:\d\d:\d\d)/); return m && (m[1] >= "{past}" || ("{past}" > "23:59" && m[1] < "00:01")) ? true : null; }})()"#), Duration::from_secs(20));
    page.eval(r#"location.hash = "series""#);
    page.wait_for(r#"document.querySelector(".list button[data-series]") ? true : null"#, Duration::from_secs(20));
    page.eval(r#"location.hash = "tables""#);
    let middle = shown_at(&mut page, "51-100/110");
    assert_eq!((middle["newer"].clone(), middle["older"].clone()), (json!(false), json!(false)), "a middle page, both neighbours live");
    // While a page is asked the pager is off: its answer held back, both buttons read off.
    page.eval(r#"(() => { window.__slowSent = false; window.__slow = "/api/table/data"; return true; })()"#);
    click(&mut page, "Older ›");
    page.wait_for("window.__slowSent || null", Duration::from_secs(10));
    assert_eq!(page.eval(r#"[...document.querySelectorAll(".list-main .pager button")].map((b) => b.disabled)"#), json!([true, true]), "the pager off while a page is asked");
    shown_at(&mut page, "101-110/110");
    click(&mut page, "‹ Newer");
    shown_at(&mut page, "51-100/110");
    // Every ordered pair of the view's own controls (a pick, Newer, Older) from that middle page,
    // the first one's answer held back so the second comes while it is out: the page shown is the
    // later pick's, else the first action's, never a page past the end nor another table's offset.
    let acts = ["pick", "newer", "older"];
    for first in acts {
        for second in acts {
            // A pick turns the pager off at once: its own page is the one drawn once Older is live.
            pick(&mut page, "sorts");
            page.wait_for(r#"(() => { const b = [...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "Older ›"); return b && !b.disabled && document.querySelector(".list-main .pager [data-rows]").dataset.rows === "1-50/110" ? true : null; })()"#, Duration::from_secs(20));
            click(&mut page, "Older ›");
            shown_at(&mut page, "51-100/110");
            let slow = if first == "pick" { "/api/tables" } else { "/api/table/data" };
            page.eval(&format!(r#"(() => {{ window.__slowSent = false; window.__slow = "{slow}"; return true; }})()"#));
            for (i, act) in [first, second].into_iter().enumerate() {
                match act {
                    "pick" => { page.eval(r#"document.querySelector('.list button[data-table="outlets"]').click()"#); }
                    "newer" => { page.eval(r#"[...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "‹ Newer").click()"#); }
                    _ => { page.eval(r#"[...document.querySelectorAll(".list-main .pager button")].find((b) => b.textContent === "Older ›").click()"#); }
                }
                if i == 0 {
                    page.wait_for("window.__slowSent || null", Duration::from_secs(10));
                }
            }
            let (table, rows) = match (first, second) {
                (_, "pick") | ("pick", _) => ("outlets", "1-4/4"),
                ("older", _) => ("sorts", "101-110/110"),
                _ => ("sorts", "1-50/110"),
            };
            let at = format!("(() => {{ const s = {shown_js}; return s && s.table === '{table}' && s.rows === '{rows}' ? s : null; }})()");
            page.wait_for(&at, Duration::from_secs(20));
            page.wait_for("window.__slowDone || null", Duration::from_secs(10));
            let s = page.eval(shown_js);
            assert_eq!((s["table"].clone(), s["rows"].clone()), (json!(table), json!(rows)), "{first} then {second}, the first's answer held back");
        }
    }
    // Two tables picked one after the other, the first one's page asked and answered last: the
    // later is the one shown.
    page.eval(r#"(() => { window.__slowSent = false; window.__slow = "name=sorts&"; document.querySelector('.list button[data-table="sorts"]').click(); return true; })()"#);
    page.wait_for("window.__slowSent || null", Duration::from_secs(10));
    page.eval(r#"document.querySelector('.list button[data-table="outlets"]').click()"#);
    page.wait_for(&format!("(() => {{ const s = {shown_js}; return s && s.table === 'outlets' ? s : null; }})()"), Duration::from_secs(20));
    page.wait_for("window.__slowDone || null", Duration::from_secs(10));
    assert_eq!(page.eval(shown_js)["table"], json!("outlets"), "the later pick stays, the earlier answer come late");
    // A refusal come late is no more drawn than an answer: for each request a pick makes (the
    // listing, then the page), a table picked with that request held back and refused, another
    // picked while it is out: the later pick's page stays, no refusal over it.
    for slow in ["/api/tables", "name=plc_main&"] {
        page.eval(&format!(r#"(() => {{ window.__slowSent = false; window.__slowRefuse = true; window.__slow = "{slow}"; document.querySelector('.list button[data-table="plc_main"]').click(); return true; }})()"#));
        page.wait_for("window.__slowSent || null", Duration::from_secs(10));
        page.eval(r#"document.querySelector('.list button[data-table="outlets"]').click()"#);
        page.wait_for(&format!("(() => {{ const s = {shown_js}; return s && s.table === 'outlets' ? s : null; }})()"), Duration::from_secs(20));
        page.wait_for("window.__slowDone || null", Duration::from_secs(10));
        let after = page.eval(&format!(r#"(() => {{ const s = {shown_js}; return [s ? s.table : null, document.querySelector(".list-main p.error")?.textContent ?? null, document.querySelector(".list button.on")?.dataset.table ?? null]; }})()"#));
        assert_eq!(after, json!(["outlets", null, "outlets"]), "{slow} refused late drew over the later pick");
    }
    // The pager off from a pick on and from the view shown again on, while the listing is out: the
    // listing held back, both buttons read off.
    let held_pager = |page: &mut Browser, act: &str| -> Value {
        page.eval(&format!(r#"(() => {{ window.__slowSent = false; window.__slow = "/api/tables"; {act}; return true; }})()"#));
        page.wait_for("window.__slowSent || null", Duration::from_secs(10));
        let off = page.eval(r#"[...document.querySelectorAll(".list-main .pager button")].map((b) => b.disabled)"#);
        page.wait_for("window.__slowDone || null", Duration::from_secs(10));
        off
    };
    pick(&mut page, "sorts");
    shown_at(&mut page, "1-50/110");
    assert_eq!(held_pager(&mut page, r#"document.querySelector('.list button[data-table="outlets"]').click()"#), json!([true, true]), "the pager off from a pick on");
    pick(&mut page, "sorts");
    shown_at(&mut page, "1-50/110");
    page.eval(r#"location.hash = "series""#);
    page.wait_for(r#"document.querySelector(".list button[data-series]") ? true : null"#, Duration::from_secs(20));
    assert_eq!(held_pager(&mut page, r#"location.hash = "tables""#), json!([true, true]), "the pager off from the view shown again on");
    shown_at(&mut page, "1-50/110");
    // The listing refused for the latest pick is said where the page was.
    page.eval(r#"(() => { window.__slowSent = false; window.__slowRefuse = true; window.__slow = "/api/tables"; document.querySelector('.list button[data-table="outlets"]').click(); return true; })()"#);
    page.wait_for("window.__slowSent || null", Duration::from_secs(10));
    let refused = page.wait_for(r#"document.querySelector(".list-main p.error")?.textContent ?? null"#, Duration::from_secs(10));
    assert_eq!(refused, json!("Error: probe refusal, late"), "the latest pick's listing refused");
    // Shown anew, the table's window takes the rows stored since; a page portal refuses is said in
    // its place.
    pick(&mut page, "sorts");
    shown_at(&mut page, "1-50/110");
    page.eval("window.__fail = true; true");
    click(&mut page, "Older ›");
    let refused = page.wait_for(r#"document.querySelector(".list-main p.error")?.textContent ?? null"#, Duration::from_secs(10));
    assert_eq!(refused, json!("Error: probe failure"));
    every_mark_shown(&mut page, "a refused page");
    // The limit the clause states: a row stamped inside the frozen window but stored after it was
    // frozen (a late line, the tail's lag) joins it and shifts its pages. The first page shown, a
    // sort stamped just before the newest ten stored, the next page opens on the first page's last.
    pick(&mut page, "sorts");
    let before_late = shown_at(&mut page, "1-50/110");
    append(&log, &[format!("{} SORT outlet=8 travel=2.0 rate=99.0", stamp(now - 1))]);
    assert!(sorts_rows(111));
    click(&mut page, "Older ›");
    let shifted = shown_at(&mut page, "51-100/111");
    assert_eq!(shifted["body"][0], before_late["body"][49], "the late row joined the frozen window and shifted its pages");
    // A pick takes the window anew, as showing the view does: a sort stored while the sorts are
    // shown, stamped at the session's now, is the newest row of the sorts picked again once the page
    // holds a status from past it.
    let late_now = store_now();
    append(&log, &[format!("{} SORT outlet=7 travel=3.0 rate=99.0", stamp(late_now))]);
    assert!(sorts_rows(112));
    let past = chrono::DateTime::from_timestamp_millis(late_now + 1_000).unwrap().format("%H:%M:%S").to_string();
    page.wait_for(&format!(r#"(() => {{ const m = (document.querySelector(".live")?.textContent ?? "").match(/(\d\d:\d\d:\d\d)/); return m && (m[1] >= "{past}" || ("{past}" > "23:59" && m[1] < "00:01")) ? true : null; }})()"#), Duration::from_secs(20));
    pick(&mut page, "sorts");
    let repicked = shown_at(&mut page, "1-50/112");
    assert_eq!(repicked["body"][0][travel], json!("3"), "the sort stored since, newest, on the sorts picked again");

    // Every request the page made went to this host and this session's own paths, its portal
    // reads the four routes under `/s/loop/portal/api/`, its stream the session's objects.
    let requests = page.requests();
    // Every request of rows and of a window names exactly what the clause says it asks (rows: the
    // table, fifty, an offset and the window's end; a window: the series whole, the hour to the
    // session's now, 600 points), each among the parameters portal's own route reads (its query's
    // struct, read from its source).
    let serve = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../veredas-portal/src/serve.rs")).unwrap();
    let fields = |name: &str| -> std::collections::BTreeSet<String> {
        let at = serve.find(&format!("pub struct {name} {{")).unwrap_or_else(|| panic!("{name}"));
        let body = &serve[at..at + serve[at..].find('}').unwrap()];
        body.lines().filter_map(|l| l.trim().strip_prefix("pub ")?.split_once(':').map(|(f, _)| f.trim().to_string())).filter(|f| !f.starts_with("struct")).collect()
    };
    let keys = |u: &str| -> std::collections::BTreeSet<String> { u.split_once('?').map(|(_, q)| q.split('&').map(|kv| kv.split('=').next().unwrap().to_string()).collect()).unwrap_or_default() };
    let set = |ks: &[&str]| ks.iter().map(|k| k.to_string()).collect::<std::collections::BTreeSet<String>>();
    let (rows_asked, windows_asked) = (fields("TableDataParams"), fields("SeriesDataParams"));
    assert!(rows_asked.contains("to") && windows_asked.contains("points"), "{rows_asked:?} {windows_asked:?}");
    let mut seen = (0, 0);
    for (u, _) in &requests {
        if u.contains("/portal/api/table/data?") {
            assert!(keys(u) == set(&["limit", "name", "offset", "to"]) && keys(u).is_subset(&rows_asked), "{u}");
            seen.0 += 1;
        }
        if u.contains("/portal/api/series/data?") {
            assert!(keys(u) == set(&["from", "points", "series", "source", "to"]) && keys(u).is_subset(&windows_asked), "{u}");
            seen.1 += 1;
        }
    }
    assert!(seen.0 > 10 && seen.1 > 3, "{seen:?}");
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

    // A table picked before the page holds any status (the object stream held back, so no now): its
    // first page asked with no window's end, its newest row's time held as the window from then on,
    // so the rows stored since join no page and its pages do not shift.
    let mut blind = Browser::open("about:blank");
    blind.call("Page.enable", json!({}));
    blind.call("Page.addScriptToEvaluateOnNewDocument", json!({ "source": "window.EventSource = class { constructor() {} addEventListener() {} close() {} };" }));
    blind.call("Page.navigate", json!({ "url": format!("http://{addr}/s/loop/#tables") }));
    blind.wait_for(r#"document.querySelector('.list button[data-table="sorts"]') ? true : null"#, Duration::from_secs(20));
    blind.eval(KEEP_ANSWERS);
    assert_eq!(blind.eval(r#"/\d\d:\d\d:\d\d/.test(document.querySelector(".live")?.textContent ?? "")"#), json!(false), "no status held");
    blind.eval(r#"document.querySelector('.list button[data-table="sorts"]').click()"#);
    let unheld = shown_at(&mut blind, "1-50/112");
    let stored = store_now();
    append(&log, &(0..10).map(|i| format!("{} SORT outlet=6 travel=2.0 rate=99.0", stamp(stored + i))).collect::<Vec<_>>());
    assert!(sorts_rows(122));
    click(&mut blind, "Older ›");
    let held = shown_at(&mut blind, "51-100/112");
    assert!(held["body"].as_array().unwrap().iter().all(|r| !unheld["body"].as_array().unwrap().contains(r)), "a row of the first page shown again");
    let answers = blind.eval("window.__answers");
    let answers = answers.as_array().unwrap();
    let first_url = answers[0]["url"].as_str().unwrap();
    let index = answers[0]["body"]["schema"].as_array().unwrap().iter().position(|c| c["name"] == "index_ms").unwrap();
    let newest = answers[0]["body"]["rows"][0][index].as_i64().unwrap();
    // (A capture rule's table carries `ts` as well, the same time as `index_ms` on every row, so
    // holding at either is one window: portal's own order prefers `index_ms`.)
    let ts_at = answers[0]["body"]["schema"].as_array().unwrap().iter().position(|c| c["name"] == "ts").unwrap();
    assert!(answers[0]["body"]["rows"].as_array().unwrap().iter().all(|r| r[index] == r[ts_at]), "{}", answers[0]["body"]["rows"][0]);
    assert_eq!((param(first_url, "to"), param(answers.last().unwrap()["url"].as_str().unwrap(), "to")), (None, Some(newest.to_string())), "{answers:?}");
    // And a table with `ts` alone for its time (the view), still with no status: held at its first
    // page's newest `ts`, as portal orders it.
    blind.eval("window.__answers = []; true");
    blind.eval(r#"document.querySelector('.list button[data-table="stamped"]').click()"#);
    let stamped_at = |rows: &str| format!("(() => {{ const s = {shown_js}; return s && s.table === 'stamped' && s.rows === '{rows}' && s.all_shown ? s : null; }})()");
    let unheld = blind.wait_for(&stamped_at("1-50/122"), Duration::from_secs(20));
    let stored = store_now();
    append(&log, &(0..10).map(|i| format!("{} SORT outlet=5 travel=2.5 rate=99.0", stamp(stored + i))).collect::<Vec<_>>());
    assert!(sorts_rows(132));
    click(&mut blind, "Older ›");
    let held = blind.wait_for(&stamped_at("51-100/122"), Duration::from_secs(20));
    assert!(held["body"].as_array().unwrap().iter().all(|r| !unheld["body"].as_array().unwrap().contains(r)), "a row of the first page shown again");
    let answers = blind.eval("window.__answers");
    let answers = answers.as_array().unwrap();
    let ts = answers[0]["body"]["schema"].as_array().unwrap().iter().position(|c| c["name"] == "ts").unwrap();
    let newest = answers[0]["body"]["rows"][0][ts].as_i64().unwrap_or_else(|| panic!("ts as a number: {}", answers[0]["body"]["rows"][0]));
    assert_eq!((param(answers[0]["url"].as_str().unwrap(), "to"), param(answers.last().unwrap()["url"].as_str().unwrap(), "to")), (None, Some(newest.to_string())), "{answers:?}");
    // Shown again with no status, on its second page: the window it held kept, so Newer brings the
    // first page back whole at the same window's end.
    blind.eval("window.__answers = []; true");
    blind.eval(r#"document.querySelector('.list button[data-table="sorts"]').click()"#);
    let first_again = shown_at(&mut blind, "1-50/132");
    click(&mut blind, "Older ›");
    shown_at(&mut blind, "51-100/132");
    blind.eval(r#"location.hash = "series""#);
    blind.wait_for(r#"document.querySelector(".list button[data-series]") ? true : null"#, Duration::from_secs(20));
    blind.eval(r#"location.hash = "tables""#);
    shown_at(&mut blind, "51-100/132");
    click(&mut blind, "‹ Newer");
    let newer = shown_at(&mut blind, "1-50/132");
    assert_eq!(newer["body"], first_again["body"], "the newest rows back, the window held");
    let answers = blind.eval("window.__answers");
    let urls: Vec<&str> = answers.as_array().unwrap().iter().map(|a| a["url"].as_str().unwrap()).collect();
    let index = answers[0]["body"]["schema"].as_array().unwrap().iter().position(|c| c["name"] == "index_ms").unwrap();
    let held_at = answers[0]["body"]["rows"][0][index].as_i64().unwrap().to_string();
    assert!(param(urls[0], "to").is_none() && urls[1..].iter().all(|u| param(u, "to").as_deref() == Some(held_at.as_str())), "{urls:?}");
    // A series picked with no status: no window asked, the chart waiting for the session's clock and
    // saying so.
    blind.eval("window.__answers = []; true");
    blind.eval(r#"location.hash = "series""#);
    blind.wait_for(r#"document.querySelector('.list button[data-series="sorter/travel"]') ? true : null"#, Duration::from_secs(20));
    blind.eval(r#"document.querySelector('.list button[data-series="sorter/travel"]').click()"#);
    let waiting = blind.wait_for(r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const s = document.querySelector(".list-main .pager strong"); return p && s && s.textContent === "sorter · travel" ? [p.dataset.points, p.textContent] : null; })()"#, Duration::from_secs(20));
    assert_eq!(waiting, json!(["waiting", "waiting for the session's clock"]));
    std::thread::sleep(Duration::from_millis(1000));
    assert_eq!(blind.eval("window.__answers.filter((a) => a.url.includes('/api/series/data')).length"), json!(0), "no window asked before the session's now");
    drop(blind);

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
    // First a sort two hours back: the travel's last hour holds no point, charted as none under
    // its own heading (after another series' chart, which it replaces).
    append(&hall_log, &[format!("{} SORT outlet=1 travel=10.5 rate=99.0", stamp(store_now() - 2 * 3_600_000))]);
    let travel_count = |n: u64| {
        wait(Duration::from_secs(20), || {
            let (_, s) = get(addr, "/s/hall/portal/api/series");
            s["series"].as_array().unwrap().iter().any(|x| x["source"] == "sorter" && x["series"] == "travel" && x["count"] == json!(n))
        })
    };
    assert!(travel_count(1));
    page.eval(r#"location.hash = "series""#);
    page.wait_for(r#"[...document.querySelectorAll(".list button[data-series]")].some((x) => x.dataset.series === "sorter/travel") || null"#, Duration::from_secs(20));
    page.eval(r#"[...document.querySelectorAll(".list button[data-series]")].find((x) => x.dataset.series === "plc/speed").click()"#);
    page.wait_for(r#"document.querySelector(".list-main .pager strong")?.textContent === "plc · speed" || null"#, Duration::from_secs(20));
    page.eval(r#"[...document.querySelectorAll(".list button[data-series]")].find((x) => x.dataset.series === "sorter/travel").click()"#);
    let none = page.wait_for(r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const head = document.querySelector(".list-main .pager strong"); return p && head && head.textContent === "sorter · travel" ? { points: p.dataset.points, said: p.textContent, line: document.querySelectorAll(".list-main svg.trend path.line").length, dots: document.querySelectorAll(".list-main svg.trend circle.point").length } : null; })()"#, Duration::from_secs(20));
    assert_eq!(none, json!({ "points": "0/0", "said": "last 60 min · 0 of 0 points · last 10.5", "line": 0, "dots": 0 }), "a window of no point");
    every_mark_shown(&mut page, "a window of no point");
    // Then one sort five minutes back: a window of one point.
    let at = store_now() - 5 * 60_000;
    append(&hall_log, &[format!("{} SORT outlet=1 travel=10.5 rate=99.0", stamp(at))]);
    assert!(travel_count(2));
    page.eval(r#"location.hash = "tables""#);
    page.eval(r#"location.hash = "series""#);
    let listed = page.wait_for(r#"(() => { const b = [...document.querySelectorAll(".list button[data-series]")]; return b.some((x) => x.dataset.series === "sorter/travel") ? b.map((x) => x.dataset.series) : null; })()"#, Duration::from_secs(20));
    let (_, s) = get(addr, "/s/hall/portal/api/series");
    let names: Vec<String> = s["series"].as_array().unwrap().iter().map(|x| format!("{}/{}", x["source"].as_str().unwrap(), x["series"].as_str().unwrap())).collect();
    assert_eq!(listed, json!(names), "hall's series, as hall's portal lists them");
    page.eval(r#"document.querySelector('.list button[data-series="sorter/travel"]').click()"#);
    let one = page.wait_for(
        &format!("(() => {{ {SHOWN_FN} const r = {}; if (r) r.shown = r.shown && shown(document.querySelector('.list-main svg.trend circle.point')); return r; }})()", r#"(() => { const p = document.querySelector(".list-main .pager [data-points]"); const dot = document.querySelector(".list-main svg.trend circle.point"); return p && dot ? { points: p.dataset.points, said: p.textContent, line: document.querySelectorAll(".list-main svg.trend path.line").length, cx: Number(dot.getAttribute("cx")), cy: Number(dot.getAttribute("cy")), shown: (() => { const c = getComputedStyle(dot), b = dot.getBoundingClientRect(); const ctx = Object.assign(document.createElement("canvas"), { width: 1, height: 1 }).getContext("2d", { willReadFrequently: true }); ctx.fillStyle = "rgba(0, 0, 0, 0)"; ctx.fillStyle = c.fill; ctx.clearRect(0, 0, 1, 1); ctx.fillRect(0, 0, 1, 1); const alpha = c.fill === "none" ? 0 : ctx.getImageData(0, 0, 1, 1).data[3] / 255; let o = 1; for (let e = dot; e; e = e.parentElement) { const s = getComputedStyle(e); o *= Number(s.opacity); if (s.display === "none") o = 0; } return b.width > 0 && b.height > 0 && c.visibility === "visible" && o > 0 && alpha * Number(c.fillOpacity) > 0 && document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2) === dot; })() } : null; })()"#),
        Duration::from_secs(20),
    );
    every_mark_shown(&mut page, "a window of one point");
    let asked: Vec<String> = page.requests().into_iter().map(|(u, _)| u).filter(|u| u.contains("/portal/api/series/data?")).collect();
    assert!(asked.iter().all(|u| u.contains("/s/hall/portal/api/series/data?")), "{asked:?}");
    let last = asked.last().unwrap().clone();
    assert!(last.contains("series=travel"), "{asked:?}");
    let query = |k: &str| last.split(['?', '&']).find_map(|kv| kv.strip_prefix(&format!("{k}="))).unwrap().to_string();
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
