//! The sort plan (contract: the sort-plan clause), from Veredas 1's `veredas-telegram` tests.

use std::collections::HashMap;

use veredas_core::telegram::sortplan::*;

fn dest(value: u16, weight: f64) -> DestinationRow {
    DestinationRow {
        value,
        weight,
        enabled: true,
        filters: HashMap::new(),
        limits: Limits::default(),
    }
}

fn parcel(object_id: u32, fields: &[(&str, &str)]) -> Parcel {
    Parcel {
        object_id,
        fields: fields
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        physical: Physical::default(),
    }
}

fn seeded(destinations: Vec<DestinationRow>, rules: Vec<RoutingRule>) -> SortPlan {
    SortPlan::new(
        PlanSettings {
            seed: Some(42),
            ..Default::default()
        },
        destinations,
        rules,
    )
}

#[test]
fn weights_are_shares_of_the_enabled_sum_not_percentages() {
    // The operator's rule: 25/25/10 means 25:25:10 of what is
    // enabled — the numbers need not add to 100 or to 1.
    let mut plan = seeded(
        vec![dest(1, 25.0), dest(2, 25.0), dest(3, 10.0)],
        Vec::new(),
    );
    let mut counts: HashMap<u16, usize> = HashMap::new();
    for i in 0..6_000 {
        let d = plan.decide(&parcel(i, &[])).expect("a decision");
        assert_eq!(d.tier, Tier::Weighted);
        *counts.entry(d.destination).or_default() += 1;
    }
    let total = 6_000_f64;
    let share = |dest: u16| counts.get(&dest).copied().unwrap_or(0) as f64 / total;
    // 25/60, 25/60, 10/60 — loose bounds, it is a simulator not a census.
    assert!((share(1) - 0.4167).abs() < 0.03, "{:?}", counts);
    assert!((share(2) - 0.4167).abs() < 0.03, "{:?}", counts);
    assert!((share(3) - 0.1667).abs() < 0.03, "{:?}", counts);
}

#[test]
fn disabled_and_zero_weight_rows_are_never_drawn() {
    let mut plan = seeded(
        vec![
            dest(1, 10.0),
            DestinationRow {
                enabled: false,
                ..dest(2, 100.0)
            },
            dest(3, 0.0),
        ],
        Vec::new(),
    );
    for i in 0..500 {
        let d = plan.decide(&parcel(i, &[])).unwrap();
        assert_eq!(d.destination, 1, "only the enabled, weighted row");
    }
}

#[test]
fn the_infeed_column_gives_each_induction_point_its_own_spread() {
    // The same chute may carry different weights per infeed, and a
    // row scoped to one infeed must never catch another's traffic.
    let row = |value, weight, infeed: &str| DestinationRow {
        filters: [("infeed".to_string(), infeed.to_string())]
            .into_iter()
            .collect(),
        ..dest(value, weight)
    };
    let mut plan = seeded(
        vec![
            row(10, 50.0, "1"),
            row(11, 50.0, "1"),
            row(20, 100.0, "2"),
            DestinationRow {
                filters: [("infeed".to_string(), "*".to_string())]
                    .into_iter()
                    .collect(),
                ..dest(99, 1.0)
            },
        ],
        Vec::new(),
    );
    let mut from_one: HashMap<u16, usize> = HashMap::new();
    for i in 0..400 {
        let d = plan.decide(&parcel(i, &[("infeed", "1")])).unwrap();
        *from_one.entry(d.destination).or_default() += 1;
    }
    assert!(from_one.contains_key(&10) && from_one.contains_key(&11));
    assert!(
        !from_one.contains_key(&20),
        "infeed 2's chute leaked: {from_one:?}"
    );
    assert!(from_one.contains_key(&99), "the * row applies everywhere");

    // Formatting differences between plants must not silently empty
    // a table: "01" and " 1 " are the same infeed as "1".
    let d = plan.decide(&parcel(999, &[("infeed", " 1 ")])).unwrap();
    assert!([10, 11, 99].contains(&d.destination), "{d:?}");

    // Fail-closed: a parcel with NO infeed captured matches only the
    // unscoped row, never another infeed's.
    let mut seen = std::collections::HashSet::new();
    for i in 0..200 {
        seen.insert(plan.decide(&parcel(i, &[])).unwrap().destination);
    }
    assert_eq!(seen, [99].into_iter().collect());
}

#[test]
fn constraints_exclude_a_chute_that_cannot_take_the_parcel() {
    let small = DestinationRow {
        limits: Limits {
            max_length_mm: Some(300),
            max_weight_g: Some(5_000),
            ..Default::default()
        },
        ..dest(1, 100.0)
    };
    let big = dest(2, 100.0);
    let mut plan = seeded(vec![small, big], Vec::new());

    let oversize = Parcel {
        physical: Physical {
            length_mm: Some(900),
            ..Default::default()
        },
        ..parcel(1, &[])
    };
    for _ in 0..200 {
        assert_eq!(plan.decide(&oversize).unwrap().destination, 2);
    }
    let heavy = Parcel {
        physical: Physical {
            weight_g: Some(9_000),
            ..Default::default()
        },
        ..parcel(2, &[])
    };
    assert_eq!(plan.decide(&heavy).unwrap().destination, 2);

    // An UNMEASURED parcel is not excluded — a plant that does not
    // measure must not divert everything to reject.
    let mut seen = std::collections::HashSet::new();
    for i in 0..300 {
        seen.insert(plan.decide(&parcel(i, &[])).unwrap().destination);
    }
    assert!(seen.contains(&1) && seen.contains(&2), "{seen:?}");
}

#[test]
fn routing_rules_beat_the_spread_and_respect_constraints_too() {
    let rules = vec![
        RoutingRule {
            field: "barcode*".into(),
            pattern: "^99".into(),
            destination: "7".into(),
            enabled: true,
        },
        RoutingRule {
            field: "id2".into(),
            pattern: "^DE".into(),
            destination: "@germany".into(),
            enabled: true,
        },
    ];
    let settings = PlanSettings {
        seed: Some(7),
        tables: [("germany".to_string(), vec![dest(30, 1.0), dest(31, 1.0)])]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    let mut plan = SortPlan::new(
        settings,
        vec![
            dest(1, 100.0),
            DestinationRow {
                limits: Limits {
                    max_weight_g: Some(1_000),
                    ..Default::default()
                },
                ..dest(7, 0.0)
            },
        ],
        rules,
    );
    assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);

    // The wildcard field reaches ANY barcode, not just the first.
    let d = plan
        .decide(&parcel(1, &[("barcode1", "12345"), ("barcode2", "990001")]))
        .unwrap();
    assert_eq!((d.destination, d.tier), (7, Tier::Routing));

    // @table indirection draws from the named table.
    let d = plan.decide(&parcel(2, &[("id2", "DE-123")])).unwrap();
    assert!([30, 31].contains(&d.destination), "{d:?}");
    assert_eq!(d.tier, Tier::Routing);

    // A routing hit that the parcel physically cannot use falls
    // through to the spread rather than sending it somewhere it
    // does not fit.
    let heavy = Parcel {
        physical: Physical {
            weight_g: Some(8_000),
            ..Default::default()
        },
        ..parcel(3, &[("barcode1", "997777")])
    };
    let d = plan.decide(&heavy).unwrap();
    assert_eq!((d.destination, d.tier), (1, Tier::Weighted));
}

#[test]
fn a_no_read_parcel_is_routable_instead_of_falling_through() {
    // The operationally critical case: no barcode was read. Without
    // an (empty) pattern it would land in the weighted spread and
    // be sorted as if it were fine.
    let mut plan = seeded(
        vec![dest(1, 100.0)],
        vec![RoutingRule {
            field: "barcode*".into(),
            pattern: EMPTY_PATTERN.into(),
            destination: "9".into(),
            enabled: true,
        }],
    );
    let d = plan.decide(&parcel(1, &[("barcode1", "")])).unwrap();
    assert_eq!((d.destination, d.tier), (9, Tier::Routing));
    let d = plan.decide(&parcel(2, &[])).unwrap();
    assert_eq!(
        d.destination, 9,
        "no barcode field at all is also a no-read"
    );
    // A parcel WITH a barcode is not caught by it.
    let d = plan.decide(&parcel(3, &[("barcode1", "123")])).unwrap();
    assert_eq!((d.destination, d.tier), (1, Tier::Weighted));
}

#[test]
fn only_decisions_actually_sent_become_sticky() {
    let mut plan = seeded(
        vec![dest(1, 100.0)],
        vec![RoutingRule {
            field: "id".into(),
            pattern: "^A".into(),
            destination: "5".into(),
            enabled: true,
        }],
    );
    let p = parcel(1, &[("id", "A100")]);

    // Decided but never remembered → the next announcement decides
    // afresh (a reply that never reached the wire must not bind).
    let first = plan.decide(&p).unwrap();
    assert_eq!(first.tier, Tier::Routing);
    let again = plan.decide(&p).unwrap();
    assert_eq!(again.tier, Tier::Routing, "not sticky until remembered");

    plan.remember(&p, &again);
    let third = plan.decide(&p).unwrap();
    assert_eq!((third.destination, third.tier), (5, Tier::Sticky));

    // A weighted draw is NEVER sticky: a spread is a spread.
    let q = parcel(2, &[("id", "B1")]);
    let weighted = plan.decide(&q).unwrap();
    assert_eq!(weighted.tier, Tier::Weighted);
    plan.remember(&q, &weighted);
    assert_eq!(plan.decide(&q).unwrap().tier, Tier::Weighted);
}

#[test]
fn a_sticky_answer_is_dropped_when_its_chute_stops_accepting() {
    let mut plan = seeded(
        vec![dest(1, 100.0), dest(5, 0.0)],
        vec![RoutingRule {
            field: "id".into(),
            pattern: "^A".into(),
            destination: "5".into(),
            enabled: true,
        }],
    );
    let p = parcel(1, &[("id", "A1")]);
    let d = plan.decide(&p).unwrap();
    plan.remember(&p, &d);
    assert_eq!(plan.decide(&p).unwrap().tier, Tier::Sticky);

    // The chute fills: the remembered answer must not be replayed
    // into a chute that would refuse the parcel.
    plan.set_blocked(5, Some("full".into()));
    let d = plan.decide(&p).unwrap();
    assert_eq!((d.destination, d.tier), (1, Tier::Weighted));

    plan.set_blocked(5, None);
    assert_eq!(plan.decide(&p).unwrap().tier, Tier::Sticky, "recovered");
}

#[test]
fn a_blocked_chute_leaves_the_spread_until_it_recovers() {
    let mut plan = seeded(vec![dest(1, 50.0), dest(2, 50.0)], Vec::new());
    plan.set_blocked(1, Some("jam".into()));
    for i in 0..300 {
        assert_eq!(plan.decide(&parcel(i, &[])).unwrap().destination, 2);
    }
    assert_eq!(plan.blocked().len(), 1);
    plan.set_blocked(1, None);
    let mut seen = std::collections::HashSet::new();
    for i in 0..300 {
        seen.insert(plan.decide(&parcel(i, &[])).unwrap().destination);
    }
    assert!(seen.contains(&1), "the chute came back");
}

#[test]
fn nothing_eligible_reaches_reject_or_reports_no_decision() {
    // Everything blocked, with a reject chute configured.
    let mut plan = SortPlan::new(
        PlanSettings {
            reject_destination: Some(999),
            seed: Some(1),
            ..Default::default()
        },
        vec![dest(1, 50.0)],
        Vec::new(),
    );
    plan.set_blocked(1, Some("full".into()));
    let d = plan.decide(&parcel(1, &[])).unwrap();
    assert_eq!((d.destination, d.tier), (999, Tier::Reject));

    // Without one, the caller is told — so it can answer the wire
    // with "ask me again" rather than inventing a chute.
    let mut plan = seeded(vec![dest(1, 50.0)], Vec::new());
    plan.set_blocked(1, Some("full".into()));
    assert_eq!(
        plan.decide(&parcel(1, &[])),
        Err(NoDecision::NothingEligible)
    );
    assert_eq!(
        SortPlan::new(PlanSettings::default(), Vec::new(), Vec::new()).decide(&parcel(1, &[])),
        Err(NoDecision::EmptyPlan)
    );
    assert_eq!(plan.counts.undecided, 1);
}

#[test]
fn broken_rules_are_warnings_that_disable_one_rule_not_the_plan() {
    // A plan is typed by hand; one bad line must not stop a plant.
    let plan = seeded(
        vec![dest(1, 10.0)],
        vec![
            RoutingRule {
                field: "id".into(),
                pattern: "([".into(),
                destination: "5".into(),
                enabled: true,
            },
            RoutingRule {
                field: "id".into(),
                pattern: "^X".into(),
                destination: "@nope".into(),
                enabled: true,
            },
            RoutingRule {
                field: "id".into(),
                pattern: "^Y".into(),
                destination: "not-a-chute".into(),
                enabled: true,
            },
            RoutingRule {
                field: "id".into(),
                pattern: "^Z".into(),
                destination: "6".into(),
                enabled: false,
            },
        ],
    );
    assert_eq!(plan.warnings.len(), 3, "{:?}", plan.warnings);
    assert!(plan.warnings[0].contains("invalid pattern"));
    assert!(plan.warnings[1].contains("@nope"));
    assert!(plan.warnings[2].contains("not-a-chute"));
    assert_eq!(plan.live_rules(), 0, "the disabled rule is not compiled");
}

#[test]
fn the_sticky_memory_stays_bounded() {
    let mut plan = SortPlan::new(
        PlanSettings {
            sticky_capacity: 100,
            seed: Some(3),
            ..Default::default()
        },
        vec![dest(1, 10.0)],
        vec![RoutingRule {
            field: "id".into(),
            pattern: ".".into(),
            destination: "5".into(),
            enabled: true,
        }],
    );
    for i in 0..1_000u32 {
        let p = parcel(i, &[("id", &format!("P{i}"))]);
        let d = plan.decide(&p).unwrap();
        plan.remember(&p, &d);
    }
    assert_eq!(
        plan.remembered(),
        100,
        "bounded at its capacity, the newest kept"
    );
}

#[test]
fn a_seeded_plan_replays_its_distribution_exactly() {
    // So a commissioning run can be reproduced.
    let draw = || {
        let mut plan = seeded(vec![dest(1, 30.0), dest(2, 70.0)], Vec::new());
        (0..50)
            .map(|i| plan.decide(&parcel(i, &[])).unwrap().destination)
            .collect::<Vec<_>>()
    };
    assert_eq!(draw(), draw());
}

#[test]
fn tier_counters_make_a_silent_fallback_visible() {
    // "The weighted tier suddenly answers everything" is how a
    // broken capture shows up; the counters are the alarm.
    let mut plan = seeded(
        vec![dest(1, 10.0)],
        vec![RoutingRule {
            field: "id".into(),
            pattern: "^A".into(),
            destination: "5".into(),
            enabled: true,
        }],
    );
    let p = parcel(1, &[("id", "A1")]);
    let d = plan.decide(&p).unwrap();
    plan.remember(&p, &d);
    plan.decide(&p).unwrap();
    plan.decide(&parcel(2, &[("id", "B")])).unwrap();
    assert_eq!(plan.counts.routing, 1);
    assert_eq!(plan.counts.sticky, 1);
    assert_eq!(plan.counts.weighted, 1);
}

// ---------------------------------------------------------------------------
// Refute round 1 (C-014): each claim pinned on an input that tells it apart.
// ---------------------------------------------------------------------------

fn rule(field: &str, pattern: &str, destination: &str) -> RoutingRule {
    RoutingRule {
        field: field.into(),
        pattern: pattern.into(),
        destination: destination.into(),
        enabled: true,
    }
}

fn scoped(value: u16, weight: f64, infeed: &str) -> DestinationRow {
    let mut row = dest(value, weight);
    row.filters.insert("infeed".into(), infeed.into());
    row
}

fn shares(plan: &mut SortPlan, n: u32, fields: &[(&str, &str)]) -> HashMap<u16, u32> {
    let mut out = HashMap::new();
    for i in 0..n {
        *out.entry(plan.decide(&parcel(i, fields)).unwrap().destination).or_default() += 1;
    }
    out
}

#[test]
fn a_rules_only_plan_decides_by_content() {
    let mut plan = seeded(vec![], vec![rule("id", "^A", "5")]);
    assert_eq!(
        plan.decide(&parcel(1, &[("id", "A1")])),
        Ok(Decision { destination: 5, tier: Tier::Routing })
    );
    assert_eq!(plan.decide(&parcel(2, &[("id", "B1")])), Err(NoDecision::NothingEligible));
    assert_eq!((plan.counts.routing, plan.counts.undecided), (1, 1));
    // With no destination, no rule and no reject chute the plan is empty.
    let mut empty = seeded(vec![], vec![]);
    assert_eq!(empty.decide(&parcel(1, &[])), Err(NoDecision::EmptyPlan));
    assert_eq!(empty.counts.undecided, 1);
}

#[test]
fn the_first_matching_rule_wins() {
    let mut plan = seeded(vec![dest(1, 1.0)], vec![rule("id", "^A", "5"), rule("id", "^AB", "6")]);
    assert_eq!(plan.decide(&parcel(1, &[("id", "AB1")])).unwrap().destination, 5);
}

#[test]
fn shares_count_only_enabled_unblocked_rows_of_the_same_infeed() {
    let mut off = dest(3, 80.0);
    off.enabled = false;
    let mut plan = seeded(vec![dest(1, 10.0), dest(2, 10.0), off, dest(4, 80.0)], vec![]);
    plan.set_blocked(4, Some("full".into()));
    let got = shares(&mut plan, 6_000, &[]);
    assert_eq!((got.get(&3), got.get(&4)), (None, None));
    let one = f64::from(got[&1]) / 6_000.0;
    assert!((one - 0.5).abs() < 0.03, "{got:?}");

    let mut plan = seeded(
        vec![scoped(1, 30.0, "1"), scoped(2, 10.0, "1"), scoped(3, 60.0, "2")],
        vec![],
    );
    let got = shares(&mut plan, 8_000, &[("infeed", "1")]);
    assert_eq!(got.get(&3), None);
    assert!((f64::from(got[&1]) / 8_000.0 - 0.75).abs() < 0.03, "{got:?}");
}

#[test]
fn an_infeed_matches_its_row_however_the_number_is_written() {
    let mut plan = seeded(vec![scoped(10, 50.0, "1"), scoped(11, 50.0, "1"), dest(99, 1.0)], vec![]);
    for infeed in ["01", " 1 ", "001", "1"] {
        let got = shares(&mut plan, 300, &[("infeed", infeed)]);
        assert!(got.contains_key(&10) && got.contains_key(&11), "{infeed}: {got:?}");
        assert!(got.get(&99).copied().unwrap_or(0) < 20, "{infeed}: {got:?}");
    }
    // Not a number: compared as text, case aside.
    let mut plan = seeded(vec![scoped(10, 1.0, "North")], vec![]);
    assert_eq!(plan.decide(&parcel(1, &[("infeed", "NORTH")])).unwrap().destination, 10);
    assert_eq!(
        plan.decide(&parcel(2, &[("infeed", "south")])),
        Err(NoDecision::NothingEligible)
    );
}

#[test]
fn fractional_weights_keep_their_shares() {
    let mut plan = seeded(vec![dest(1, 1e-6), dest(2, 3e-6)], vec![]);
    let got = shares(&mut plan, 40_000, &[]);
    assert!((f64::from(got[&1]) / 40_000.0 - 0.25).abs() < 0.02, "{got:?}");
    let mut plan = seeded(vec![dest(1, 1e-7), dest(2, 1e-7)], vec![]);
    let got = shares(&mut plan, 10_000, &[]);
    assert!((f64::from(got[&1]) / 10_000.0 - 0.5).abs() < 0.03, "{got:?}");
    // A weight that is not a finite positive number is never drawn.
    let mut plan = seeded(
        vec![dest(1, 10.0), dest(2, f64::INFINITY), dest(3, f64::NAN), dest(4, -5.0)],
        vec![],
    );
    assert_eq!(shares(&mut plan, 500, &[]), HashMap::from([(1, 500)]));
}

#[test]
fn a_broken_rule_disables_only_itself() {
    let mut plan = seeded(
        vec![dest(1, 1.0)],
        vec![rule("id", "(", "5"), rule("id", "^OK", "6"), rule("id", ".", "@nope")],
    );
    assert_eq!((plan.live_rules(), plan.warnings.len()), (1, 2), "{:?}", plan.warnings);
    assert_eq!(
        plan.decide(&parcel(1, &[("id", "OK1")])),
        Ok(Decision { destination: 6, tier: Tier::Routing })
    );
}

#[test]
fn a_remembered_table_answer_keeps_the_limits_of_its_table() {
    let mut settings = PlanSettings { seed: Some(42), ..Default::default() };
    let mut light = dest(30, 1.0);
    light.limits.max_weight_g = Some(1_000);
    settings.tables.insert("germany".into(), vec![light, dest(31, 1.0)]);
    let mut plan = SortPlan::new(settings, vec![dest(1, 100.0)], vec![rule("id2", "^DE", "@germany")]);
    // Find a parcel that draws chute 30 while unmeasured, and send it there.
    let mut found = None;
    for i in 0..100 {
        let p = parcel(i, &[("id2", &format!("DE{i}"))]);
        let d = plan.decide(&p).unwrap();
        assert_eq!(d.tier, Tier::Routing);
        if d.destination == 30 {
            plan.remember(&p, &d);
            found = Some(p);
            break;
        }
    }
    let mut p = found.expect("chute 30 is drawn within 100 parcels");
    assert_eq!(plan.decide(&p).unwrap(), Decision { destination: 30, tier: Tier::Sticky });
    // Announced again at 8 kg: chute 30 cannot take it, so the remembered answer is not reused.
    p.physical.weight_g = Some(8_000);
    assert_eq!(plan.decide(&p).unwrap(), Decision { destination: 31, tier: Tier::Routing });
}

#[test]
fn the_sticky_memory_keeps_the_newest_and_keys_on_the_named_field() {
    let mut plan = SortPlan::new(
        PlanSettings { sticky_capacity: 100, seed: Some(3), ..Default::default() },
        vec![dest(1, 10.0)],
        vec![rule("id", ".", "5")],
    );
    for i in 0..1_000u32 {
        let p = parcel(i, &[("id", &format!("P{i}"))]);
        let d = plan.decide(&p).unwrap();
        plan.remember(&p, &d);
    }
    let mut tier =
        |i: u32| plan.decide(&parcel(i, &[("id", &format!("P{i}"))])).unwrap().tier;
    assert_eq!(
        [tier(999), tier(900), tier(899), tier(0)],
        [Tier::Sticky, Tier::Sticky, Tier::Routing, Tier::Routing]
    );

    let mut plan = SortPlan::new(
        PlanSettings { sticky_key: "id2".into(), seed: Some(3), ..Default::default() },
        vec![dest(1, 10.0)],
        vec![rule("id2", "^X", "5")],
    );
    let first = parcel(1, &[("id2", "X7")]);
    let d = plan.decide(&first).unwrap();
    plan.remember(&first, &d);
    // A second announcement under another object id, the same id2: the same parcel.
    assert_eq!(plan.decide(&parcel(2, &[("id2", "X7")])).unwrap().tier, Tier::Sticky);
}

#[test]
fn every_limit_excludes_and_a_value_at_the_limit_is_admitted() {
    let size = |w, l, h, g, v| Physical {
        width_mm: w,
        length_mm: l,
        height_mm: h,
        weight_g: g,
        volume: v,
    };
    let over = [
        (Limits { max_width_mm: Some(100), ..Default::default() }, size(Some(101), None, None, None, None)),
        (Limits { max_length_mm: Some(100), ..Default::default() }, size(None, Some(101), None, None, None)),
        (Limits { max_height_mm: Some(100), ..Default::default() }, size(None, None, Some(101), None, None)),
        (Limits { max_weight_g: Some(100), ..Default::default() }, size(None, None, None, Some(101), None)),
        (Limits { max_volume: Some(100), ..Default::default() }, size(None, None, None, None, Some(101))),
    ];
    for (limits, physical) in over {
        let mut row = dest(7, 1.0);
        row.limits = limits;
        let mut plan = seeded(vec![row], vec![]);
        let mut p = parcel(1, &[]);
        p.physical = physical;
        assert_eq!(plan.decide(&p), Err(NoDecision::NothingEligible), "{limits:?}");
        let mut p = parcel(2, &[]);
        p.physical = size(Some(100), Some(100), Some(100), Some(100), Some(100));
        assert_eq!(plan.decide(&p).unwrap().destination, 7, "at the limit: {limits:?}");
    }
}

#[test]
fn the_reject_counter_counts() {
    let mut row = dest(1, 1.0);
    row.limits.max_weight_g = Some(10);
    let mut plan = SortPlan::new(
        PlanSettings { reject_destination: Some(99), seed: Some(1), ..Default::default() },
        vec![row],
        vec![],
    );
    let mut heavy = parcel(1, &[]);
    heavy.physical.weight_g = Some(50);
    assert_eq!(plan.decide(&heavy).unwrap(), Decision { destination: 99, tier: Tier::Reject });
    assert_eq!(plan.counts, TierCounts { reject: 1, ..Default::default() });
}

// ---------------------------------------------------------------------------
// Refute round 2 (C-014).
// ---------------------------------------------------------------------------

fn keyed_plan() -> SortPlan {
    SortPlan::new(
        PlanSettings { sticky_key: "id2".into(), seed: Some(3), ..Default::default() },
        vec![dest(1, 10.0)],
        vec![rule("id", "^A", "5")],
    )
}

#[test]
fn a_blank_or_overlapping_sticky_key_is_no_shared_identity() {
    for blank in ["", "   "] {
        let mut plan = keyed_plan();
        let first = parcel(1, &[("id", "A1"), ("id2", blank)]);
        let d = plan.decide(&first).unwrap();
        assert_eq!(d, Decision { destination: 5, tier: Tier::Routing });
        plan.remember(&first, &d);
        // Another parcel with a blank id2 is not the same parcel: its own content decides.
        let other = parcel(2, &[("id", "B1"), ("id2", blank)]);
        assert_eq!(plan.decide(&other).unwrap(), Decision { destination: 1, tier: Tier::Weighted });
        // The first, announced again, is still itself (by its object id).
        assert_eq!(plan.decide(&first).unwrap().tier, Tier::Sticky);
    }
    // Object 77 without id2 and another parcel whose id2 is "77" are different keys.
    let mut plan = keyed_plan();
    let seventy_seven = parcel(77, &[("id", "A7")]);
    let d = plan.decide(&seventy_seven).unwrap();
    plan.remember(&seventy_seven, &d);
    assert_eq!(
        plan.decide(&parcel(9, &[("id", "B9"), ("id2", "77")])).unwrap(),
        Decision { destination: 1, tier: Tier::Weighted }
    );
}

#[test]
fn a_remembered_table_answer_is_judged_by_its_own_table_only() {
    // Chute 30 is unlimited in the main table, limited to 1 kg in the table the rule draws from.
    let mut settings = PlanSettings { seed: Some(42), ..Default::default() };
    let mut light = dest(30, 1.0);
    light.limits.max_weight_g = Some(1_000);
    settings.tables.insert("germany".into(), vec![light, dest(31, 1.0)]);
    let mut plan = SortPlan::new(
        settings,
        vec![dest(1, 50.0), dest(30, 50.0)],
        vec![rule("id2", "^DE", "@germany")],
    );
    let mut found = None;
    for i in 0..100 {
        let p = parcel(i, &[("id2", &format!("DE{i}"))]);
        let d = plan.decide(&p).unwrap();
        if d.destination == 30 {
            plan.remember(&p, &d);
            found = Some(p);
            break;
        }
    }
    let mut p = found.expect("chute 30 is drawn within 100 parcels");
    p.physical.weight_g = Some(8_000);
    assert_eq!(plan.decide(&p).unwrap(), Decision { destination: 31, tier: Tier::Routing });
}

#[test]
fn a_direct_rule_is_judged_by_the_main_table_and_trusted_when_unlisted_there() {
    // Chute 30 appears only in a named table, scoped there to infeed 2 and disabled for 40: a
    // rule naming the chute itself is the operator's explicit word, as in Veredas 1.
    let mut settings = PlanSettings { seed: Some(1), ..Default::default() };
    let mut off = dest(40, 1.0);
    off.enabled = false;
    settings.tables.insert("germany".into(), vec![scoped(30, 1.0, "2"), off]);
    let mut plan = SortPlan::new(
        settings,
        vec![dest(1, 1.0)],
        vec![rule("id", "^U", "30"), rule("id", "^V", "40")],
    );
    assert_eq!(
        plan.decide(&parcel(1, &[("id", "U1"), ("infeed", "1")])).unwrap(),
        Decision { destination: 30, tier: Tier::Routing }
    );
    assert_eq!(
        plan.decide(&parcel(2, &[("id", "V1")])).unwrap(),
        Decision { destination: 40, tier: Tier::Routing }
    );
}

#[test]
fn an_answer_remembered_again_is_the_newest() {
    let mut settings = PlanSettings { sticky_capacity: 2, seed: Some(5), ..Default::default() };
    settings.tables.insert("t".into(), vec![dest(30, 1.0), dest(31, 1.0)]);
    let mut plan = SortPlan::new(settings, vec![dest(1, 1.0)], vec![rule("id", "^X", "@t")]);
    let decide_and_remember = |plan: &mut SortPlan, p: &Parcel| {
        let d = plan.decide(p).unwrap();
        assert_eq!(d.tier, Tier::Routing);
        plan.remember(p, &d);
        d.destination
    };
    let (a, b, c) = (parcel(1, &[("id", "XA")]), parcel(2, &[("id", "XB")]), parcel(3, &[("id", "XC")]));
    let first = decide_and_remember(&mut plan, &a);
    decide_and_remember(&mut plan, &b);
    // A's chute blocks: A is decided afresh and remembered again, now the newest.
    plan.set_blocked(first, Some("full".into()));
    assert_ne!(decide_and_remember(&mut plan, &a), first);
    plan.set_blocked(first, None);
    decide_and_remember(&mut plan, &c);
    // Capacity 2: the oldest, B, went; A and C stay.
    let tier = |plan: &mut SortPlan, p: &Parcel| plan.decide(p).unwrap().tier;
    assert_eq!(
        [tier(&mut plan, &a), tier(&mut plan, &c), tier(&mut plan, &b)],
        [Tier::Sticky, Tier::Sticky, Tier::Routing]
    );
}

#[test]
fn huge_finite_weights_keep_their_shares() {
    let mut plan = seeded(vec![dest(1, 1e308), dest(2, 1e308)], vec![]);
    let got = shares(&mut plan, 4_000, &[]);
    assert!((f64::from(got[&1]) / 4_000.0 - 0.5).abs() < 0.04, "{got:?}");
    let mut plan = seeded(vec![dest(1, f64::MAX), dest(2, f64::MAX / 3.0)], vec![]);
    let got = shares(&mut plan, 8_000, &[]);
    assert!((f64::from(got[&1]) / 8_000.0 - 0.75).abs() < 0.03, "{got:?}");
}
