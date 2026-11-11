//! The dashboards' panels, computed from a session's store into exactly what the page draws.
//!
//! Every number a panel shows is computed here, against the windows and pages portal's store
//! serves, so a test asserts it exactly; the page lays the numbers out. A window is the last
//! `minutes` up to `to` on the store's time axis (the plant's clock read as UTC); `to` is the
//! store's now unless the caller names it.

use std::collections::BTreeMap;

use serde_json::{Value as Json, json};
use veredas_core::SeriesRef;
use veredas_portal::serve::lttb;
use veredas_portal::session::Live;
use veredas_portal::store::{MAX_PAGE, RowsQuery};

use crate::binding::{Aggregate, Binding, CELLS_BINS, CELLS_ROWS_MAX, Column, Limit, Object, Panel, PanelKind};
use crate::feed::{ObjectView, Snapshot};
use crate::state::{Reading, StateClass, format_number};

/// The points a trend line is drawn with, and a sparkline.
pub const TREND_POINTS: usize = 300;
pub const SPARK_POINTS: usize = 60;

const MINUTE_MS: i64 = 60_000;

/// How far before its window a timeline looks for the state holding at the window's start (a
/// state logged on change lasts until the next sample, however long ago it came).
pub const TIMELINE_LOOKBACK_MS: i64 = 24 * 60 * MINUTE_MS;

fn window(minutes: u32, to: i64) -> (i64, i64) {
    (to - minutes as i64 * MINUTE_MS, to)
}

fn points(live: &Live, o: &Object, from: i64, to: i64) -> Vec<(i64, f64)> {
    live.store
        .query_numeric(&SeriesRef::new(&o.value.source, &o.value.series), o.value.key.as_deref(), from, to)
        .into_iter()
        .map(|p| (p.index_ms, p.value))
        .collect()
}

fn pairs(pts: &[(i64, f64)]) -> Json {
    Json::Array(pts.iter().map(|(t, v)| json!([t, v])).collect())
}

fn view_json(o: &Object, v: Option<&ObjectView>) -> Json {
    match v {
        Some(v) => json!({
            "id": o.id, "label": o.label, "unit": o.unit, "text": v.text,
            "state": v.state.name(), "mark": v.state.mark(), "word": v.word, "stale": v.stale,
            "since_ms": v.since_ms, "level": v.level,
        }),
        None => json!({ "id": o.id, "label": o.label, "unit": o.unit, "text": null, "state": "stale", "mark": StateClass::Stale.mark(), "word": "no value", "stale": true }),
    }
}

/// The sub-line of a value tile: how long ago a stale value arrived, the rule that holds, or how
/// far a normal value is from its target.
pub fn tile_sub(o: &Object, v: Option<&ObjectView>, target: Option<f64>, now_ms: i64) -> String {
    let Some(v) = v else { return "no value".to_string() };
    if v.stale {
        return match v.since_ms {
            Some(t) => format!("last value {} s ago", ((now_ms - t).max(0)) / 1000),
            None => "no value".to_string(),
        };
    }
    if v.state != StateClass::Normal {
        // The rule that named the state, described: `below 98 %`.
        if let Some(r) = o.rules.iter().find(|r| r.class == v.state && r.word == v.word) {
            return r.compare.describe(&o.unit);
        }
        return v.word.clone();
    }
    match (target, v.value.as_ref().and_then(Reading::as_f64)) {
        (Some(t), Some(x)) if t != 0.0 => {
            let pct = (x - t) / t.abs() * 100.0;
            let sign = if pct >= 0.0 { "+" } else { "" };
            format!("{sign}{}% on the {} target", format_number(pct, Some(0)), format_number(t, None))
        }
        _ => String::new(),
    }
}

/// The state a sample of the object's own series reads as, by the rules over its own value.
fn state_of(o: &Object, x: f64) -> (StateClass, String) {
    let r = Reading::Num(x);
    match o.rules.iter().filter(|r| r.series.is_none()).find(|rule| rule.compare.holds(&r)) {
        Some(rule) => (rule.class, rule.word.clone()),
        None => (StateClass::Normal, o.normal.clone()),
    }
}

/// Runs of one state over a window: each sample's state, consecutive equal ones merged, a run
/// lasting until the next sample (the last one until `to`).
pub fn segments(o: &Object, pts: &[(i64, f64)], to: i64) -> Vec<Json> {
    let mut out: Vec<(i64, i64, StateClass, String)> = Vec::new();
    for (i, (t, x)) in pts.iter().enumerate() {
        let (class, word) = state_of(o, *x);
        let end = pts.get(i + 1).map(|p| p.0).unwrap_or(to);
        match out.last_mut() {
            Some(last) if last.2 == class && last.3 == word => last.1 = end,
            _ => out.push((*t, end, class, word)),
        }
    }
    out.into_iter().map(|(from, to, c, w)| json!({ "from": from, "to": to, "state": c.name(), "mark": c.mark(), "word": w })).collect()
}

/// Clock-aligned buckets of `bucket_ms` covering `[from, to]`, each aggregated; the bucket
/// holding `to` is the running one.
pub fn buckets(pts: &[(i64, f64)], from: i64, to: i64, bucket_ms: i64, agg: Aggregate) -> Vec<Json> {
    let first = from.div_euclid(bucket_ms) * bucket_ms;
    let mut out = Vec::new();
    let mut start = first;
    while start <= to {
        let end = start + bucket_ms;
        let vals: Vec<f64> = pts.iter().filter(|(t, _)| *t >= start && *t < end).map(|(_, v)| *v).collect();
        let value = if vals.is_empty() {
            if agg == Aggregate::Count { Some(0.0) } else { None }
        } else {
            Some(match agg {
                Aggregate::Mean => vals.iter().sum::<f64>() / vals.len() as f64,
                Aggregate::Sum => vals.iter().sum(),
                Aggregate::Count => vals.len() as f64,
                Aggregate::Min => vals.iter().cloned().fold(f64::INFINITY, f64::min),
                Aggregate::Max => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
                Aggregate::Last => *vals.last().unwrap_or(&0.0),
            })
        };
        out.push(json!({ "start": start, "end": end, "value": value, "n": vals.len(), "running": to < end }));
        start = end;
    }
    out
}

/// The median of values (the mean of the two middle ones for an even count).
pub fn median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    Some(if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0 })
}

/// The population standard deviation.
pub fn sigma(vals: &[f64]) -> Option<f64> {
    if vals.is_empty() {
        return None;
    }
    let mean = vals.iter().sum::<f64>() / vals.len() as f64;
    Some((vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / vals.len() as f64).sqrt())
}

/// A distribution of values in `bins` equal bins from `center - spread` to `center + spread`
/// (the last bin closed), with the counts that fall below and above it.
pub fn distribution(vals: &[f64], center: f64, spread: f64, bins: usize) -> (Vec<u64>, u64, u64) {
    let (lo, hi) = (center - spread, center + spread);
    let width = (hi - lo) / bins as f64;
    let mut counts = vec![0u64; bins];
    let (mut below, mut above) = (0, 0);
    for v in vals {
        if *v < lo {
            below += 1;
        } else if *v > hi {
            above += 1;
        } else {
            let i = (((v - lo) / width).floor() as usize).min(bins - 1);
            counts[i] += 1;
        }
    }
    (counts, below, above)
}

fn cell_text(v: &Json, decimals: Option<u8>) -> String {
    match v {
        Json::Null => String::new(),
        Json::Number(n) => n.as_f64().map(|x| format_number(x, decimals)).unwrap_or_else(|| n.to_string()),
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn reading_of(v: &Json) -> Option<Reading> {
    match v {
        Json::Number(n) => n.as_f64().map(Reading::Num),
        Json::String(s) => Some(Reading::Text(s.clone())),
        Json::Bool(b) => Some(Reading::Num(if *b { 1.0 } else { 0.0 })),
        _ => None,
    }
}

/// The key of a row as text: a number as shown, text as it is.
fn key_text(v: &Json) -> Option<String> {
    match v {
        Json::Null => None,
        Json::Number(n) => n.as_f64().map(|x| format_number(x, None)),
        Json::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Rows sorted by their column `k`: numbers as numbers, anything else as its text, and nulls
/// last whichever way the rest goes.
pub fn sort_rows(rows: &mut [Vec<Json>], k: usize, desc: bool) {
    rows.sort_by(|a, b| {
        let o = compare_cells(&a[k], &b[k]);
        if desc && !a[k].is_null() && !b[k].is_null() { o.reverse() } else { o }
    });
}

fn compare_cells(a: &Json, b: &Json) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (a, b) {
        (Json::Null, Json::Null) => Ordering::Equal,
        (Json::Null, _) => Ordering::Greater,
        (_, Json::Null) => Ordering::Less,
        (Json::Number(x), Json::Number(y)) => x.as_f64().unwrap_or(0.0).partial_cmp(&y.as_f64().unwrap_or(0.0)).unwrap_or(Ordering::Equal),
        _ => cell_text(a, None).cmp(&cell_text(b, None)),
    }
}

fn table_panel(live: &Live, table: &str, columns: &[Column], sort: &Option<(String, bool)>, limit: usize) -> Result<Json, String> {
    let page = live.store.query_rows(table, &RowsQuery { from_ms: None, to_ms: None, limit, offset: 0 }).map_err(|e| e.to_string())?;
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    let chosen: Vec<Column> = if columns.is_empty() {
        names.iter().map(|n| Column { name: n.to_string(), label: n.to_string(), bar: false, decimals: None, rules: Vec::new() }).collect()
    } else {
        columns.to_vec()
    };
    let mut idx = Vec::new();
    for c in &chosen {
        let i = names.iter().position(|n| *n == c.name).ok_or_else(|| format!("`{table}` has no column `{}` (its columns: {})", c.name, names.join(", ")))?;
        idx.push(i);
    }
    let mut rows: Vec<Vec<Json>> = page.rows.iter().map(|r| idx.iter().map(|i| r.get(*i).cloned().unwrap_or(Json::Null)).collect()).collect();
    if let Some((col, desc)) = sort {
        let Some(k) = chosen.iter().position(|c| &c.name == col) else {
            return Err(format!("the sort column `{col}` is not shown"));
        };
        sort_rows(&mut rows, k, *desc);
    }
    let maxes: Vec<f64> = (0..chosen.len()).map(|k| rows.iter().filter_map(|r| r[k].as_f64()).fold(0.0_f64, |m, v| m.max(v.abs()))).collect();
    let out_rows: Vec<Json> = rows
        .iter()
        .map(|r| {
            Json::Array(
                chosen
                    .iter()
                    .enumerate()
                    .map(|(k, c)| {
                        let v = &r[k];
                        let mut cell = json!({ "text": cell_text(v, c.decimals), "value": v });
                        if c.bar {
                            cell["bar"] = match v.as_f64() {
                                Some(x) if maxes[k] > 0.0 => json!((x.abs() / maxes[k]).clamp(0.0, 1.0)),
                                _ => json!(0.0),
                            };
                        }
                        if !c.rules.is_empty() {
                            let reading = reading_of(v);
                            let hit = reading.as_ref().and_then(|x| c.rules.iter().find(|rule| rule.compare.holds(x)));
                            let (class, word) = match hit {
                                Some(rule) => (rule.class, rule.word.clone()),
                                None => (StateClass::Normal, String::new()),
                            };
                            cell["state"] = json!(class.name());
                            cell["mark"] = json!(class.mark());
                            cell["word"] = json!(word);
                        }
                        cell
                    })
                    .collect(),
            )
        })
        .collect();
    Ok(json!({
        "columns": chosen.iter().map(|c| json!({ "name": c.name, "label": c.label, "bar": c.bar })).collect::<Vec<_>>(),
        "rows": out_rows,
        "total": page.total,
    }))
}

/// Charts in cells over a window: the rows read newest first in pages of `page_rows`, at most `cap` of
/// them (the route reads pages of the store's largest and at most [`CELLS_ROWS_MAX`]).
#[allow(clippy::too_many_arguments)]
pub fn cells(live: &Live, table: &str, key: &str, value: &str, minutes: u32, bucket_minutes: u32, spread: f64, to: i64, page_rows: usize, cap: usize) -> Result<Json, String> {
    let (from, to) = window(minutes, to);
    let mut rows: Vec<Vec<Json>> = Vec::new();
    let mut columns: Vec<String> = Vec::new();
    let mut total: Option<u64> = None;
    // Paged newest first by stamp: each page reads the rows at or before the last stamp taken,
    // past those already read at that stamp. A row stored between two reads stamped after the
    // last one taken never shifts a page, where an offset would read one row twice and miss
    // another. One stored at the very stamp a page ended on sorts ahead of the rows read there
    // (the store breaks a tie by arrival, newest first), so the next page starts that many rows
    // early: never past a row not yet read, and the rows read again are known by their source and
    // sequence and taken once. Every base table carries the three columns this needs; a view
    // without one of them is refused, its rows having nothing to be read once by.
    let mut at: Option<(usize, usize, usize)> = None;
    let mut taken: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let (mut bound, mut tied) = (to, 0usize);
    loop {
        let q = RowsQuery { from_ms: Some(from), to_ms: Some(bound), limit: page_rows, offset: tied };
        let page = live.store.query_rows(table, &q).map_err(|e| e.to_string())?;
        let (ti, si, qi) = match at {
            Some(at) => at,
            None => {
                columns = page.columns.iter().map(|c| c.name.clone()).collect();
                let has = |name: &str| columns.iter().position(|c| c == name);
                match (has("index_ms"), has("source"), has("seq")) {
                    (Some(ti), Some(si), Some(qi)) => *at.insert((ti, si, qi)),
                    _ => {
                        let missing: Vec<String> = ["index_ms", "source", "seq"].into_iter().filter(|n| has(n).is_none()).map(|n| format!("`{n}`")).collect();
                        return Err(format!(
                            "`{table}` has no {} (cells read each row once by `index_ms`, `source` and `seq`; its columns: {})",
                            missing.join(", "),
                            columns.join(", ")
                        ));
                    }
                }
            }
        };
        // The window as it stood at the first read.
        total.get_or_insert(page.total);
        let n = page.rows.len();
        for r in page.rows {
            match r.get(ti).and_then(Json::as_i64) {
                Some(t) if t != bound => (bound, tied) = (t, 1),
                _ => tied += 1,
            }
            let id = (r.get(si).map(Json::to_string).unwrap_or_default(), r.get(qi).map(Json::to_string).unwrap_or_default());
            if taken.insert(id) {
                rows.push(r);
            }
        }
        if n < page_rows || rows.len() >= cap {
            break;
        }
    }
    let total = total.unwrap_or(0);
    rows.truncate(cap);
    let find = |name: &str| columns.iter().position(|c| c == name).ok_or_else(|| format!("`{table}` has no column `{name}` (its columns: {})", columns.join(", ")));
    let (ki, vi) = (find(key)?, find(value)?);
    let ti = columns.iter().position(|c| c == "index_ms");
    let mut groups: BTreeMap<String, Vec<(Option<i64>, f64)>> = BTreeMap::new();
    for r in &rows {
        let (Some(k), Some(v)) = (r.get(ki).and_then(key_text), r.get(vi).and_then(Json::as_f64)) else { continue };
        let t = ti.and_then(|i| r.get(i)).and_then(Json::as_i64);
        groups.entry(k).or_default().push((t, v));
    }
    let mut keys: Vec<String> = groups.keys().cloned().collect();
    // Keys that all read as numbers sort as numbers; any other set sorts as text.
    if keys.iter().all(|k| k.parse::<f64>().is_ok()) {
        keys.sort_by(|a, b| a.parse::<f64>().unwrap_or(0.0).partial_cmp(&b.parse::<f64>().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal));
    }
    let bucket_ms = bucket_minutes as i64 * MINUTE_MS;
    let out: Vec<Json> = keys
        .iter()
        .map(|k| {
            let g = &groups[k];
            let mut vals: Vec<f64> = g.iter().map(|(_, v)| *v).collect();
            vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let med = median(&vals).unwrap_or(0.0);
            let (bins, below, above) = distribution(&vals, med, spread, CELLS_BINS);
            // The median per clock-aligned bucket, for the sparkline.
            let mut per: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
            for (t, v) in g {
                if let Some(t) = t {
                    per.entry(t.div_euclid(bucket_ms) * bucket_ms).or_default().push(*v);
                }
            }
            let spark: Vec<Json> = per
                .into_iter()
                .map(|(start, mut vs)| {
                    vs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                    json!([start, median(&vs)])
                })
                .collect();
            json!({ "key": k, "n": vals.len(), "median": med, "sigma": sigma(&vals), "spark": spark, "bins": bins, "below": below, "above": above })
        })
        .collect();
    Ok(json!({ "from": from, "to": to, "spread": spread, "rows": out, "read": rows.len(), "total": total, "capped": total as usize > rows.len() }))
}

fn panel_body(live: &Live, b: &Binding, snap: &Snapshot, p: &Panel, to: i64, now_unix: i64) -> Result<Json, String> {
    let view = |i: usize| snap.objects.get(i).filter(|v| v.id == b.objects[i].id);
    Ok(match &p.kind {
        PanelKind::Value { object, minutes, target } => {
            let o = &b.objects[*object];
            let (from, to) = window(*minutes, to);
            let spark = lttb::downsample(&points(live, o, from, to), SPARK_POINTS);
            let mut j = view_json(o, view(*object));
            j["sub"] = json!(tile_sub(o, view(*object), *target, now_unix));
            j["target"] = json!(target);
            j["spark"] = pairs(&spark);
            j["from"] = json!(from);
            j["to"] = json!(to);
            j
        }
        PanelKind::Trend { object, minutes, limit } => {
            let o = &b.objects[*object];
            let (from, to) = window(*minutes, to);
            let raw = points(live, o, from, to);
            let breaches: Vec<(i64, f64)> = match limit {
                Some(Limit::Above(x)) => raw.iter().filter(|(_, v)| v > x).cloned().collect(),
                Some(Limit::Below(x)) => raw.iter().filter(|(_, v)| v < x).cloned().collect(),
                None => Vec::new(),
            };
            let line = lttb::downsample(&raw, TREND_POINTS);
            json!({
                "object": o.id, "unit": o.unit, "from": from, "to": to, "total": raw.len(),
                "points": pairs(&line), "breaches": pairs(&breaches),
                "limit": match limit { Some(Limit::Above(x)) => json!({ "above": x }), Some(Limit::Below(x)) => json!({ "below": x }), None => Json::Null },
            })
        }
        PanelKind::Bars { object, minutes, bucket_minutes, aggregate, target } => {
            let o = &b.objects[*object];
            let (from, to) = window(*minutes, to);
            let bucket_ms = *bucket_minutes as i64 * MINUTE_MS;
            let first = from.div_euclid(bucket_ms) * bucket_ms;
            let raw = points(live, o, first, to);
            json!({ "object": o.id, "unit": o.unit, "from": first, "to": to, "buckets": buckets(&raw, from, to, bucket_ms, *aggregate), "target": target })
        }
        PanelKind::Timeline { objects, minutes } => {
            let (from, to) = window(*minutes, to);
            let rows: Vec<Json> = objects
                .iter()
                .map(|i| {
                    let o = &b.objects[*i];
                    let mut pts = points(live, o, from, to);
                    // The state holding at the window's start: the last sample of the day before
                    // it, its run opened at `from`.
                    if pts.first().is_none_or(|p| p.0 > from)
                        && let Some(&(_, v)) = points(live, o, from - TIMELINE_LOOKBACK_MS, from - 1).last()
                    {
                        pts.insert(0, (from, v));
                    }
                    json!({ "id": o.id, "label": o.label, "segments": segments(o, &pts, to), "now": view_json(o, view(*i)) })
                })
                .collect();
            json!({ "from": from, "to": to, "rows": rows })
        }
        PanelKind::Grid { objects } => {
            let cells: Vec<Json> = objects.iter().map(|i| view_json(&b.objects[*i], view(*i))).collect();
            // The heading's count, computed here as every figure the page draws: the cells in any
            // state but normal.
            let not_normal = cells.iter().filter(|c| c["state"] != "normal").count();
            json!({ "cells": cells, "not_normal": not_normal })
        }
        PanelKind::Levels { objects, ticks } => {
            let rows: Vec<Json> = objects
                .iter()
                .map(|i| {
                    let o = &b.objects[*i];
                    let mut j = view_json(o, view(*i));
                    let frac = |x: f64| ((x - o.range.0) / (o.range.1 - o.range.0)).clamp(0.0, 1.0);
                    j["fraction"] = json!(view(*i).and_then(|v| v.value.as_ref()).and_then(Reading::as_f64).map(frac));
                    j["ticks"] = json!(ticks.iter().map(|t| frac(*t)).collect::<Vec<_>>());
                    j
                })
                .collect();
            json!({ "rows": rows })
        }
        PanelKind::Table { table, columns, sort, limit } => table_panel(live, table, columns, sort, *limit)?,
        PanelKind::Cells { table, key, value, minutes, bucket_minutes, spread } => cells(live, table, key, value, *minutes, *bucket_minutes, *spread, to, MAX_PAGE, CELLS_ROWS_MAX)?,
    })
}

/// A dashboard's panels computed: every panel's kind, title and span with its numbers, or its
/// error in its place (one panel's failure never hides the others).
pub fn dashboard(live: &Live, b: &Binding, snap: &Snapshot, index: usize, to: i64, now_unix: i64) -> Option<Json> {
    let d = b.dashboards.get(index)?;
    let panels: Vec<Json> = d
        .panels
        .iter()
        .map(|p| {
            let mut head = json!({ "kind": p.kind.name(), "title": p.title, "span": p.span });
            match panel_body(live, b, snap, p, to, now_unix) {
                Ok(body) => head["data"] = body,
                Err(e) => head["error"] = json!(e),
            }
            head
        })
        .collect();
    Some(json!({ "name": d.name, "columns": d.columns, "to": to, "panels": panels }))
}

