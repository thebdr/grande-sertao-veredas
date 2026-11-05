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
