//! The sort plan: how a parcel becomes a destination.
//!
//! The plan is **operator-maintained plant data**, not code — CSVs the
//! sorting supervisor edits in Excel or from trajetorias's web page:
//!
//! ```text
//! plan/
//!   destinations.csv   value,weight,enabled,infeed,max_w,max_l,max_h,max_weight,max_vol
//!   routing.csv        field,pattern,destination,enabled
//!   plan.json          reject destination, sticky policy, table wiring
//! ```
//!
//! Two tiers, in order (the operator's structure):
//!
//! 1. **Information-based** (`routing.csv`) — content decides: a barcode
//!    prefix, an id pattern, a customer code. Ordered, first match wins,
//!    and the answer is **sticky**: the same parcel announced at a second
//!    scan point gets the same destination.
//! 2. **Weighted spread** (`destinations.csv`) — the fallback: draw
//!    among the eligible destinations, each weight a share of the SUM OF
//!    ENABLED weights (no base-100 normalization). Drawn **fresh** each
//!    time, per the operator: a spread is a spread.
//!
//! **Constraints filter both tiers.** A destination whose limits the
//! parcel exceeds is ineligible everywhere — an information-based hit
//! that cannot physically take the parcel falls through rather than
//! sending it there. When nothing is eligible, the plan's named reject
//! destination answers; when there is no plan at all, the caller sends
//! the wire's "ask me again" rather than inventing a chute.
//!
//! Everything here is pure: no I/O, no clock, no sockets. Loading and
//! reload live in [`crate::plansource`]; the socket side calls
//! [`SortPlan::decide`] and gets a [`Decision`] it can encode.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::rng::Pcg32;

/// A parcel, as the plan sees it: the fields captured from the wire plus
/// its physical size. Field names are free-form so a plan can match on
/// anything the profile captured (`id`, `id2`, `barcode1`… `infeed`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Parcel {
    /// The MSC's object id — the identity everything is keyed by.
    pub object_id: u32,
    /// Captured fields, e.g. `infeed` → `"2"`, `barcode1` → `"991234"`.
    pub fields: HashMap<String, String>,
    pub physical: Physical,
}

/// Physical measurements from the induction line (mm / g / device units).
/// `None` = not measured, which never excludes a destination — an
/// unmeasured parcel must not silently vanish into reject.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Physical {
    pub width_mm: Option<u32>,
    pub length_mm: Option<u32>,
    pub height_mm: Option<u32>,
    pub weight_g: Option<u32>,
    pub volume: Option<u32>,
}

impl Parcel {
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    /// Fields whose name starts with `prefix` — how `barcode*` reaches
    /// all sixteen barcodes without naming each.
    pub fn fields_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> {
        self.fields
            .iter()
            .filter(move |(k, _)| k.starts_with(prefix))
            .map(|(_, v)| v.as_str())
    }
}

/// One destination row of `destinations.csv`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DestinationRow {
    /// The chute number the MSC understands.
    pub value: u16,
    /// Share of the sum of ENABLED weights (not a percentage).
    #[serde(default)]
    pub weight: f64,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Filter columns: `infeed` → `"2"`, or `*`/empty for "any". Matched
    /// against the parcel's captured fields.
    #[serde(default)]
    pub filters: HashMap<String, String>,
    /// Physical limits; `None` = unlimited.
    #[serde(default)]
    pub limits: Limits,
}

fn yes() -> bool {
    true
}

/// What a destination can physically take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_width_mm: Option<u32>,
    pub max_length_mm: Option<u32>,
    pub max_height_mm: Option<u32>,
    pub max_weight_g: Option<u32>,
    pub max_volume: Option<u32>,
}

impl Limits {
    /// Whether a parcel fits. An unmeasured dimension never excludes:
    /// the plant may not measure at all, and a silent diversion of every
    /// parcel to reject would be the worst possible failure.
    pub fn admits(&self, p: &Physical) -> bool {
        let ok = |limit: Option<u32>, value: Option<u32>| match (limit, value) {
            (Some(limit), Some(value)) => value <= limit,
            _ => true,
        };
        ok(self.max_width_mm, p.width_mm)
            && ok(self.max_length_mm, p.length_mm)
            && ok(self.max_height_mm, p.height_mm)
            && ok(self.max_weight_g, p.weight_g)
            && ok(self.max_volume, p.volume)
    }
}

/// One row of `routing.csv` — the information-based tier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingRule {
    /// A captured field name, or `prefix*` to mean "any field starting
    /// with prefix" (e.g. `barcode*`).
    pub field: String,
    /// A regex over that field's value. The special value `(empty)`
    /// matches a missing or blank field — how a **no-read** parcel is
    /// routed, the case that must never reach the weighted spread.
    pub pattern: String,
    /// A chute number, or `@table` to draw from a named weighted table.
    pub destination: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// The pattern that means "this field is absent or blank".
pub const EMPTY_PATTERN: &str = "(empty)";

/// A compiled routing rule (its regex built once).
struct CompiledRule {
    field: String,
    /// `Some` when the field name ended in `*`.
    prefix: Option<String>,
    matcher: Matcher,
    destination: String,
}

enum Matcher {
    /// Matches a missing or blank field.
    Empty,
    Regex(regex::Regex),
}

/// What the plan decided, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub destination: u16,
    pub tier: Tier,
}

/// Which tier produced a decision — recorded in the audit so the
/// distribution can be read honestly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// An information-based rule matched (and was replayed if sticky).
    Routing,
    /// Remembered from this parcel's earlier announcement.
    Sticky,
    /// The weighted spread.
    Weighted,
    /// Nothing was eligible; the plan's reject destination.
    Reject,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Routing => "routing rule",
            Tier::Sticky => "sticky (already decided)",
            Tier::Weighted => "weighted spread",
            Tier::Reject => "reject (nothing eligible)",
        }
    }
}

/// Why no decision could be made — the caller answers the wire with
/// "ask me again" rather than inventing a chute.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NoDecision {
    /// Nothing eligible and no reject destination configured.
    NothingEligible,
    /// The plan has no destinations at all.
    EmptyPlan,
}

/// The plan's non-CSV settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanSettings {
    /// Where parcels go when nothing else is eligible. Without it, an
    /// undecidable parcel gets "ask me again" instead.
    #[serde(default)]
    pub reject_destination: Option<u16>,
    /// Which captured field identifies a parcel across scan points.
    /// Empty = the object id.
    #[serde(default)]
    pub sticky_key: String,
    /// How many decisions to remember.
    #[serde(default = "default_sticky_capacity")]
    pub sticky_capacity: usize,
    /// Optional fixed seed, so a commissioning run's distribution can be
    /// replayed exactly.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Extra named weighted tables a routing rule can reach with
    /// `@name` (the main table is `destinations`).
    #[serde(default)]
    pub tables: HashMap<String, Vec<DestinationRow>>,
}

fn default_sticky_capacity() -> usize {
    10_000
}

impl Default for PlanSettings {
    fn default() -> PlanSettings {
        PlanSettings {
            reject_destination: None,
            sticky_key: String::new(),
            sticky_capacity: default_sticky_capacity(),
            seed: None,
            tables: HashMap::new(),
        }
    }
}

/// How deep `@table` indirection may go before the plan is judged
/// circular. A rules file is typed by hand, so a cycle will happen.
const MAX_TABLE_DEPTH: usize = 8;

/// A loaded, compiled sort plan.
pub struct SortPlan {
    settings: PlanSettings,
    destinations: Vec<DestinationRow>,
    rules: Vec<CompiledRule>,
    /// Chutes currently refusing parcels (FULL, JAM, blocked…), learned
    /// from BINSTATUS and CHUTE_FULL outcomes.
    blocked: HashMap<u16, String>,
    sticky: StickyMemory,
    rng: Pcg32,
    /// Problems found while compiling — surfaced, never fatal.
    pub warnings: Vec<String>,
    /// Per-tier counters, so "the fallback suddenly answers everything"
    /// is visible rather than silent.
    pub counts: TierCounts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TierCounts {
    pub routing: u64,
    pub sticky: u64,
    pub weighted: u64,
    pub reject: u64,
    pub undecided: u64,
}

impl SortPlan {
    /// Compile a plan. Bad regexes and unknown `@table` references are
    /// warnings that disable a single rule — never a refusal to load,
    /// because a plant with a half-valid plan still has to sort.
    pub fn new(
        settings: PlanSettings,
        destinations: Vec<DestinationRow>,
        rules: Vec<RoutingRule>,
    ) -> SortPlan {
        let mut warnings = Vec::new();
        let mut compiled = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            if !rule.enabled {
                continue;
            }
            let matcher = if rule.pattern.trim() == EMPTY_PATTERN {
                Matcher::Empty
            } else {
                match regex::Regex::new(&rule.pattern) {
                    Ok(re) => Matcher::Regex(re),
                    Err(e) => {
                        warnings.push(format!(
                            "routing rule {} (`{}`) has an invalid pattern and is ignored: {e}",
                            i + 1,
                            rule.pattern
                        ));
                        continue;
                    }
                }
            };
            let dest = rule.destination.trim();
            if let Some(table) = dest.strip_prefix('@') {
                if table != "destinations" && !settings.tables.contains_key(table) {
                    warnings.push(format!(
                        "routing rule {} points at unknown table `@{table}` and is ignored",
                        i + 1
                    ));
                    continue;
                }
            } else if dest.parse::<u16>().is_err() {
                warnings.push(format!(
                    "routing rule {} has destination `{dest}`, which is neither a chute number nor @table; ignored",
                    i + 1
                ));
                continue;
            }
            let field = rule.field.trim().to_string();
            let prefix = field.strip_suffix('*').map(str::to_string);
            compiled.push(CompiledRule {
                field,
                prefix,
                matcher,
                destination: dest.to_string(),
            });
        }
        if destinations.is_empty() {
            warnings.push("the plan has no destinations".into());
        }
        let seed = settings.seed;
        let capacity = settings.sticky_capacity;
        SortPlan {
            settings,
            destinations,
            rules: compiled,
            blocked: HashMap::new(),
            sticky: StickyMemory::new(capacity),
            rng: match seed {
                Some(seed) => Pcg32::new(seed, 1),
                None => Pcg32::from_entropy(1),
            },
            warnings,
            counts: TierCounts::default(),
        }
    }

    /// Mark a chute as unable to take parcels (BINSTATUS FULL/JAM, a
    /// CHUTE_FULL outcome), or clear it when it recovers.
    pub fn set_blocked(&mut self, destination: u16, reason: Option<String>) {
        match reason {
            Some(reason) => {
                self.blocked.insert(destination, reason);
            }
            None => {
                self.blocked.remove(&destination);
            }
        }
    }

    pub fn blocked(&self) -> &HashMap<u16, String> {
        &self.blocked
    }

    /// Remember a decision as actually SENT.
    ///
    /// Stickiness is recorded here rather than inside `decide` on
    /// purpose: a decision that never reached the wire must not bind the
    /// parcel's next announcement.
    pub fn remember(&mut self, parcel: &Parcel, decision: &Decision) {
        if decision.tier == Tier::Routing {
            let key = self.sticky_key(parcel);
            self.sticky.put(key, decision.destination);
        }
    }

    fn sticky_key(&self, parcel: &Parcel) -> String {
        let name = self.settings.sticky_key.trim();
        if name.is_empty() {
            return parcel.object_id.to_string();
        }
        parcel
            .field(name)
            .map(str::to_string)
            .unwrap_or_else(|| parcel.object_id.to_string())
    }

    /// Decide where a parcel goes.
    pub fn decide(&mut self, parcel: &Parcel) -> Result<Decision, NoDecision> {
        if self.destinations.is_empty() && self.settings.reject_destination.is_none() {
            self.counts.undecided += 1;
            return Err(NoDecision::EmptyPlan);
        }

        // 1. Already decided for this parcel? Only if it is still able
        //    to take it — a chute that filled since must not be reused.
        let key = self.sticky_key(parcel);
        if let Some(dest) = self.sticky.get(&key)
            && self.eligible_dest(dest, parcel)
        {
            self.counts.sticky += 1;
            return Ok(Decision {
                destination: dest,
                tier: Tier::Sticky,
            });
        }

        // 2. Information-based rules, in order.
        if let Some(decision) = self.routing_decision(parcel) {
            self.counts.routing += 1;
            return Ok(decision);
        }

        // 3. The weighted spread over what is eligible.
        if let Some(dest) = self.draw_from("destinations", parcel, 0) {
            self.counts.weighted += 1;
            return Ok(Decision {
                destination: dest,
                tier: Tier::Weighted,
            });
        }

        // 4. Nothing eligible: the reject chute if the plan names one.
        match self.settings.reject_destination {
            Some(dest) => {
                self.counts.reject += 1;
                Ok(Decision {
                    destination: dest,
                    tier: Tier::Reject,
                })
            }
            None => {
                self.counts.undecided += 1;
                Err(NoDecision::NothingEligible)
            }
        }
    }

    fn routing_decision(&mut self, parcel: &Parcel) -> Option<Decision> {
        // Which rule matched, resolved before touching &mut self.
        let hit = self.rules.iter().position(|rule| rule.matches(parcel))?;
        let destination = self.rules[hit].destination.clone();
        match destination.strip_prefix('@') {
            Some(table) => {
                let table = table.to_string();
                self.draw_from(&table, parcel, 0)
                    .map(|destination| Decision {
                        destination,
                        tier: Tier::Routing,
                    })
            }
            None => {
                let dest: u16 = destination.parse().ok()?;
                // A routing hit still has to be physically possible —
                // constraints filter BOTH tiers.
                self.eligible_dest(dest, parcel).then_some(Decision {
                    destination: dest,
                    tier: Tier::Routing,
                })
            }
        }
    }

    /// Is this specific chute able to take this parcel right now?
    fn eligible_dest(&self, dest: u16, parcel: &Parcel) -> bool {
        if self.blocked.contains_key(&dest) {
            return false;
        }
        // Rows are per (destination, filter) — the parcel may match any
        // row naming this chute.
        let mut named = false;
        for row in &self.destinations {
            if row.value != dest {
                continue;
            }
            named = true;
            if row.enabled && row.limits.admits(&parcel.physical) && filters_match(row, parcel) {
                return true;
            }
        }
        // A routing rule may name a chute the destinations table does
        // not list at all; trust the operator's explicit rule, but never
        // trust a blocked chute (checked above).
        !named
    }

    /// Draw from a weighted table over the eligible rows.
    fn draw_from(&mut self, table: &str, parcel: &Parcel, depth: usize) -> Option<u16> {
        if depth > MAX_TABLE_DEPTH {
            return None;
        }
        let rows: Vec<DestinationRow> = if table == "destinations" {
            self.destinations.clone()
        } else {
            self.settings.tables.get(table)?.clone()
        };
        let eligible: Vec<&DestinationRow> = rows
            .iter()
            .filter(|row| {
                row.enabled
                    && row.weight > 0.0
                    && !self.blocked.contains_key(&row.value)
                    && row.limits.admits(&parcel.physical)
                    && filters_match(row, parcel)
            })
            .collect();
        let total: f64 = eligible.iter().map(|row| row.weight).sum();
        if total <= 0.0 {
            return None;
        }
        // A share of the SUM OF ENABLED weights — the operator's rule,
        // no normalization to 100.
        let mut point = self.rng.uniform(0.0, total, 6);
        for row in &eligible {
            if point <= row.weight {
                return Some(row.value);
            }
            point -= row.weight;
        }
        eligible.last().map(|row| row.value)
    }

    pub fn destinations(&self) -> &[DestinationRow] {
        &self.destinations
    }

    pub fn settings(&self) -> &PlanSettings {
        &self.settings
    }

    /// How many routing rules compiled and are live (enabled, valid, resolvable).
    pub fn live_rules(&self) -> usize {
        self.rules.len()
    }

    /// How many parcels the sticky memory holds now (never more than its capacity).
    pub fn remembered(&self) -> usize {
        self.sticky.map.len()
    }
}

/// Do a row's filter columns match this parcel?
///
/// `*` and blank mean "any"; comparison is trimmed and
/// case-insensitive, because `infeed=01` and `infeed=1` come from
/// different plants' formatting of the same thing. A filter naming a
/// field the parcel does not carry EXCLUDES the row (fail-closed): a
/// row scoped to infeed 2 must not catch traffic whose infeed is
/// unknown.
fn filters_match(row: &DestinationRow, parcel: &Parcel) -> bool {
    row.filters.iter().all(|(column, want)| {
        let want = want.trim();
        if want.is_empty() || want == "*" {
            return true;
        }
        match parcel.field(column) {
            Some(have) => have.trim().eq_ignore_ascii_case(want),
            None => false,
        }
    })
}

impl CompiledRule {
    fn matches(&self, parcel: &Parcel) -> bool {
        match (&self.prefix, &self.matcher) {
            // `barcode*` with (empty): true when NO barcode field has a
            // value — the no-read parcel.
            (Some(prefix), Matcher::Empty) => !parcel
                .fields_with_prefix(prefix)
                .any(|v| !v.trim().is_empty()),
            (Some(prefix), Matcher::Regex(re)) => {
                parcel.fields_with_prefix(prefix).any(|v| re.is_match(v))
            }
            (None, Matcher::Empty) => parcel
                .field(&self.field)
                .map(|v| v.trim().is_empty())
                .unwrap_or(true),
            (None, Matcher::Regex(re)) => parcel
                .field(&self.field)
                .map(|v| re.is_match(v))
                .unwrap_or(false),
        }
    }
}

/// Bounded key → destination memory: the same parcel, announced again,
/// gets the answer it already got.
struct StickyMemory {
    capacity: usize,
    map: HashMap<String, u16>,
    /// Insertion order, for eviction.
    order: std::collections::VecDeque<String>,
}

impl StickyMemory {
    fn new(capacity: usize) -> StickyMemory {
        StickyMemory {
            capacity: capacity.max(1),
            map: HashMap::new(),
            order: std::collections::VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<u16> {
        self.map.get(key).copied()
    }

    fn put(&mut self, key: String, dest: u16) {
        if self.map.insert(key.clone(), dest).is_none() {
            self.order.push_back(key);
            while self.order.len() > self.capacity {
                if let Some(old) = self.order.pop_front() {
                    self.map.remove(&old);
                }
            }
        }
    }
}
