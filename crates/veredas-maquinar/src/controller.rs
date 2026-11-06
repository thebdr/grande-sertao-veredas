//! The simulated controller: its behaviour as a versioned rules file, and the engine that runs it.
//!
//! What a controller does on its wire is configuration, never code (`docs/questions.md` Q8): a
//! `veredas-controller` file (`docs/controller-schema.md`) names, by the messages of a catalog,
//! the **items** it announces at a rate and awaits an answer for (an announcement, a reply keyed
//! by a field, an outcome after a travel time, drawn from a weighted mix or forced by the
//! destination's state), the **answers** it gives to incoming messages (fields copied, constant,
//! drawn, or decided by rules), the **registers** its cyclic messages read and its **commands**
//! set, and its **destinations** with named state bits that block. The same file plays either
//! side of a conversation: the machine, or the stand-in for the unit that answers it.
//!
//! [`Controller`] is pure and tick-driven: [`Controller::tick`] advances its clock and returns
//! what is due; [`Controller::on_message`] takes a decoded message and returns what it answers.
//! No socket, no sleep, no wall clock but the one a `now` field asks for: a test drives it
//! exactly, and the bench channel ([`crate::channel`]) drives it from the wire.

use std::collections::{BTreeMap, HashMap, VecDeque};

use chrono::Local;
use serde::Deserialize;
use veredas_core::rng::Pcg32;
use veredas_core::telegram::PeerId;
use veredas_core::telegram::catalog::Catalog;
use veredas_core::telegram::compose::{Compose, ComposeSpec};

/// The format name every controller file states.
pub const FORMAT: &str = "veredas-controller";
/// The one version this reader knows.
pub const VERSION: u32 = 1;

/// How many finished items the status keeps.
pub const HISTORY: usize = 200;
/// How many decided keys a sticky decision remembers.
pub const STICKY: usize = 10_000;

// ---------------------------------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------------------------------

/// A value as a file writes it: a number, a flag or text; every field is text on the wire.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Literal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Text(String),
}

impl Literal {
    pub fn text(&self) -> String {
        match self {
            Literal::Int(i) => i.to_string(),
            Literal::Float(f) => f.to_string(),
            Literal::Bool(b) => b.to_string(),
            Literal::Text(t) => t.clone(),
        }
    }
}

/// One field of a message the controller writes, and where its value comes from: exactly one
/// of `value`, `range`, `counter`, `cycle`, `template`, `from`, `register`, `now` or `decide`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSpec {
    pub name: String,
    /// A constant.
    #[serde(default)]
    pub value: Option<Literal>,
    /// A draw within `[low, high]`, whole at zero `decimals`.
    #[serde(default)]
    pub range: Option<[f64; 2]>,
    #[serde(default)]
    pub decimals: u8,
    /// A number counting up from `first`, back to it after `last`.
    #[serde(default)]
    pub counter: Option<Counter>,
    /// The next of these, round and round.
    #[serde(default)]
    pub cycle: Option<Vec<Literal>>,
    /// Text with `{name}` placeholders of the fields before it and of the context.
    #[serde(default)]
    pub template: Option<String>,
    /// A field of the context: the incoming message's, the item's, the destination's.
    #[serde(default)]
    pub from: Option<String>,
    /// A register's value.
    #[serde(default)]
    pub register: Option<String>,
    /// The wall clock in this chrono format.
    #[serde(default)]
    pub now: Option<String>,
    /// A destination decided by rules.
    #[serde(default)]
    pub decide: Option<Decide>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counter {
    #[serde(default = "one")]
    pub first: u64,
    #[serde(default)]
    pub last: Option<u64>,
}

fn one() -> u64 {
    1
}

/// A destination by rules: first-match patterns, then a weighted spread over what is not
/// blocked, then a reject value; sticky per key when asked.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decide {
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub spread: Vec<Share>,
    #[serde(default)]
    pub reject: Option<Literal>,
    /// The context field whose value keys a remembered decision.
    #[serde(default)]
    pub sticky: Option<String>,
}

/// A first-match rule: `pattern` is a regex over `field`, or `(empty)` for a missing or blank
/// field.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub field: String,
    pub pattern: String,
    pub value: Literal,
}

/// The pattern that means "this field is absent or blank".
pub const EMPTY_PATTERN: &str = "(empty)";

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Share {
    pub value: Literal,
    pub weight: f64,
}

/// The destinations the controller serves, each with a state word of named bits.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationsFile {
    pub list: Vec<u64>,
    /// Named bits of the state word.
    #[serde(default)]
    pub bits: BTreeMap<String, u32>,
    /// The bits a destination rests at.
    #[serde(default)]
    pub resting: Vec<String>,
    /// Bits set at the start, by destination (keys are numbers written as text).
    #[serde(default)]
    pub initial: BTreeMap<String, Vec<String>>,
    /// Which bits refuse an item, in order of precedence, and the outcome fields they force.
    #[serde(default)]
    pub blocking: Vec<Blocking>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blocking {
    pub bit: String,
    #[serde(default)]
    pub set: BTreeMap<String, Literal>,
    /// The item comes round again instead of ending.
    #[serde(default)]
    pub requeue: bool,
}

/// The items the controller announces.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemsFile {
    /// The announcement message.
    pub message: String,
    pub interval_ms: u64,
    /// The announced field that identifies an item.
    pub key: String,
    /// A register that must be non-zero for the controller to announce.
    #[serde(default)]
    pub gate: Option<String>,
    /// How long a requeued item takes to come round again.
    #[serde(default = "lap")]
    pub lap_ms: u64,
    pub fields: Vec<FieldSpec>,
    #[serde(default)]
    pub faults: Vec<Fault>,
    #[serde(default)]
    pub reply: Option<ReplyFile>,
    #[serde(default)]
    pub outcome: Option<OutcomeFile>,
}

fn lap() -> u64 {
    30_000
}

/// A named fault: this share of items has these fields set instead.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fault {
    pub name: String,
    #[serde(default)]
    pub percent: u8,
    pub set: BTreeMap<String, Literal>,
}

/// The reply the controller awaits for an item.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyFile {
    pub message: String,
    /// The reply's field holding the item's key; the items' key name when absent.
    #[serde(default)]
    pub key: Option<String>,
    /// The reply's field naming the destination.
    #[serde(default)]
    pub destination: Option<String>,
    /// Dispositions by a field's value, in order; the first match decides, else `deliver`.
    #[serde(default)]
    pub when: Vec<Disposition>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Disposition {
    pub field: String,
    pub equals: Literal,
    pub action: Action,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// The item travels to its destination and ends in an outcome.
    Deliver,
    /// The item comes round again after a lap.
    Requeue,
    /// The item is parked: asked about no more.
    Park,
}

/// What the controller reports when an item's travel ends.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeFile {
    pub message: String,
    #[serde(default)]
    pub after_ms: u64,
    pub fields: Vec<FieldSpec>,
    /// The weighted mix of outcomes, when the destination does not force one.
    #[serde(default)]
    pub mix: Vec<MixEntry>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MixEntry {
    pub name: String,
    pub weight: f64,
    #[serde(default)]
    pub set: BTreeMap<String, Literal>,
    #[serde(default)]
    pub requeue: bool,
}

/// An answer to an incoming message.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerFile {
    pub on: String,
    pub send: String,
    #[serde(default)]
    pub after_ms: u64,
    pub fields: Vec<FieldSpec>,
}

/// A command: an incoming message whose field's value sets registers.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandFile {
    pub on: String,
    pub field: String,
    pub when: Vec<Setting>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub equals: Literal,
    pub set: BTreeMap<String, i64>,
}

/// A message sent on an interval.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CyclicFile {
    pub message: String,
    pub every_ms: u64,
    /// One message per destination, with `destination` and `state` in the context.
    #[serde(default)]
    pub per_destination: bool,
    pub fields: Vec<FieldSpec>,
}

#[derive(Deserialize)]
struct Header {
    format: Option<String>,
    version: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    version: u32,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    registers: BTreeMap<String, i64>,
    #[serde(default)]
    destinations: Option<DestinationsFile>,
    #[serde(default)]
    items: Option<ItemsFile>,
    #[serde(default)]
    answers: Vec<AnswerFile>,
    #[serde(default)]
    commands: Vec<CommandFile>,
    #[serde(default)]
    cyclic: Vec<CyclicFile>,
}

/// Why a controller file was refused, or a message could not be built.
#[derive(Debug, thiserror::Error)]
pub enum RulesError {
    #[error("not a controller file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `{FORMAT}`")]
    WrongFormat(String),
    #[error("version {0} of `{FORMAT}` is not known here (this reader knows {VERSION})")]
    UnknownVersion(u32),
    #[error("{0}")]
    Invalid(String),
}

fn invalid(msg: impl Into<String>) -> RulesError {
    RulesError::Invalid(msg.into())
}

// ---------------------------------------------------------------------------------------------
// Fields, compiled
// ---------------------------------------------------------------------------------------------

/// Where a field's value comes from, compiled.
#[derive(Clone, Debug)]
enum Source {
    Value(String),
    Range { low: f64, high: f64, decimals: u8 },
    Counter { first: u64, last: Option<u64>, next: u64 },
    Cycle { values: Vec<String>, at: usize },
    Template(Compose),
    From(String),
    Register(String),
    Now(String),
    Decide(CompiledDecide),
}

#[derive(Clone, Debug)]
struct CompiledDecide {
    rules: Vec<(String, Option<regex::Regex>, String)>,
    spread: Vec<(String, f64)>,
    reject: Option<String>,
    sticky: Option<String>,
    remembered: HashMap<String, String>,
    order: VecDeque<String>,
}

/// One compiled field.
#[derive(Clone, Debug)]
struct Field {
    name: String,
    source: Source,
}

/// A list of fields that renders a message's values, keeping the state its counters and cycles
/// need.
#[derive(Clone, Debug)]
pub struct Fields {
    fields: Vec<Field>,
}

/// What a field may draw on when it renders.
pub struct Context<'a> {
    /// The fields of what this message is about: the incoming message, the item, the
    /// destination.
    pub fields: &'a HashMap<String, String>,
    pub registers: &'a BTreeMap<String, i64>,
    /// The destinations that refuse an item right now, for a spread to avoid.
    pub blocked: &'a [u64],
    pub rng: &'a mut Pcg32,
}

/// Where a field list sits, for what its sources may draw on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// A plain channel's cyclic message: no context, no registers.
    Plain,
    /// The controller's announcement: the item's earlier fields.
    Items,
    /// An answer: the incoming message's fields.
    Answer,
    /// An outcome: the item's fields and its reply's.
    Outcome,
    /// A cyclic message: the registers, and the destination when per destination.
    Cyclic { per_destination: bool },
}

impl Fields {
    /// Compile a list, refusing a field with no source or more than one, an empty name twice,
    /// a range that is empty or not finite, a counter ending before it starts, an empty cycle, a
    /// bad template, a source its place does not take, a register that does not exist, a decide
    /// with a bad pattern or nothing to decide with; and, when a message is given, a list that
    /// leaves one of the message's names unwritten.
    pub fn compile(place: &str, specs: &[FieldSpec], at: Place, registers: &BTreeMap<String, i64>, needs: Option<&[String]>) -> Result<Fields, RulesError> {
        let mut fields = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        for (i, s) in specs.iter().enumerate() {
            let where_ = format!("{place}.fields[{i}]");
            if s.name.trim().is_empty() {
                return Err(invalid(format!("{where_}: the field has no name")));
            }
            if names.contains(&s.name.as_str()) {
                return Err(invalid(format!("{where_}: two fields are named `{}`", s.name)));
            }
            let where_ = format!("{where_} (`{}`)", s.name);
            let sources = [
                s.value.is_some(),
                s.range.is_some(),
                s.counter.is_some(),
                s.cycle.is_some(),
                s.template.is_some(),
                s.from.is_some(),
                s.register.is_some(),
                s.now.is_some(),
                s.decide.is_some(),
            ]
            .iter()
            .filter(|b| **b)
            .count();
            if sources != 1 {
                return Err(invalid(format!(
                    "{where_}: exactly one of value, range, counter, cycle, template, from, register, now or decide ({sources} given)"
                )));
            }
            if s.decimals > 15 {
                return Err(invalid(format!("{where_}: decimals is {}, over 15", s.decimals)));
            }
            let source = if let Some(v) = &s.value {
                Source::Value(v.text())
            } else if let Some([low, high]) = s.range {
                if !low.is_finite() || !high.is_finite() {
                    return Err(invalid(format!("{where_}: the range is not finite")));
                }
                if low > high {
                    return Err(invalid(format!("{where_}: the range is empty ({low} > {high})")));
                }
                Source::Range { low, high, decimals: s.decimals }
            } else if let Some(c) = &s.counter {
                if let Some(last) = c.last
                    && last < c.first
                {
                    return Err(invalid(format!("{where_}: the counter ends at {last}, before its first {}", c.first)));
                }
                Source::Counter { first: c.first, last: c.last, next: c.first }
            } else if let Some(c) = &s.cycle {
                if c.is_empty() {
                    return Err(invalid(format!("{where_}: the cycle is empty")));
                }
                Source::Cycle { values: c.iter().map(Literal::text).collect(), at: 0 }
            } else if let Some(t) = &s.template {
                let compose = Compose::new(&ComposeSpec { template: Some(t.clone()), ..Default::default() })
                    .map_err(|e| invalid(format!("{where_}: {e}")))?;
                Source::Template(compose)
            } else if let Some(f) = &s.from {
                if at == Place::Plain {
                    return Err(invalid(format!("{where_}: `from` needs a message or an item to read from")));
                }
                if let Place::Cyclic { per_destination } = at
                    && !(per_destination && (f == "destination" || f == "state"))
                {
                    return Err(invalid(format!("{where_}: a cyclic message reads `destination` or `state`, per destination")));
                }
                Source::From(f.clone())
            } else if let Some(r) = &s.register {
                if at == Place::Plain {
                    return Err(invalid(format!("{where_}: a plain channel has no registers")));
                }
                if !registers.contains_key(r) {
                    return Err(invalid(format!("{where_}: no register `{r}`")));
                }
                Source::Register(r.clone())
            } else if let Some(n) = &s.now {
                let mut probe = String::new();
                use std::fmt::Write as _;
                if write!(probe, "{}", Local::now().format(n)).is_err() {
                    return Err(invalid(format!("{where_}: `{n}` is not a datetime format")));
                }
                Source::Now(n.clone())
            } else if let Some(d) = &s.decide {
                if at != Place::Answer {
                    return Err(invalid(format!("{where_}: `decide` answers a message")));
                }
                let mut rules = Vec::new();
                for (j, r) in d.rules.iter().enumerate() {
                    let re = if r.pattern.trim() == EMPTY_PATTERN {
                        None
                    } else {
                        Some(regex::Regex::new(&r.pattern).map_err(|e| invalid(format!("{where_}.decide.rules[{j}]: {e}")))?)
                    };
                    rules.push((r.field.clone(), re, r.value.text()));
                }
                for (j, sh) in d.spread.iter().enumerate() {
                    if !sh.weight.is_finite() || sh.weight < 0.0 {
                        return Err(invalid(format!("{where_}.decide.spread[{j}]: a weight is not a finite, non-negative number")));
                    }
                }
                if rules.is_empty() && d.spread.is_empty() && d.reject.is_none() {
                    return Err(invalid(format!("{where_}.decide: nothing to decide with")));
                }
                Source::Decide(CompiledDecide {
                    rules,
                    spread: d.spread.iter().map(|sh| (sh.value.text(), sh.weight)).collect(),
                    reject: d.reject.as_ref().map(Literal::text),
                    sticky: d.sticky.clone(),
                    remembered: HashMap::new(),
                    order: VecDeque::new(),
                })
            } else {
                unreachable!("one source was counted")
            };
            names.push(&s.name);
            fields.push(Field { name: s.name.clone(), source });
        }
        if let Some(needs) = needs {
            for n in needs {
                if !names.contains(&n.as_str()) {
                    return Err(invalid(format!("{place}: the message needs `{n}`, which no field writes")));
                }
            }
        }
        Ok(Fields { fields })
    }

    /// The field names, in order.
    pub fn names(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.name.clone()).collect()
    }

    /// Render every field, in order, each later one seeing the earlier ones. `None` when a
    /// decision could not be made or a value was missing; the note says why.
    pub fn render(&mut self, ctx: &mut Context<'_>) -> Result<HashMap<String, String>, String> {
        let mut out: HashMap<String, String> = HashMap::new();
        for field in &mut self.fields {
            let value = match &mut field.source {
                Source::Value(v) => v.clone(),
                Source::Range { low, high, decimals } => {
                    if *decimals == 0 {
                        ctx.rng.int_in(low.ceil() as i64, high.floor().max(low.ceil()) as i64).to_string()
                    } else {
                        format!("{:.*}", *decimals as usize, ctx.rng.uniform(*low, *high, *decimals))
                    }
                }
                Source::Counter { first, last, next } => {
                    let v = *next;
                    *next = match last {
                        Some(last) if v >= *last => *first,
                        _ => v.wrapping_add(1),
                    };
                    v.to_string()
                }
                Source::Cycle { values, at } => {
                    let v = values[*at % values.len()].clone();
                    *at = (*at + 1) % values.len();
                    v
                }
                Source::Template(compose) => {
                    let mut values = ctx.fields.clone();
                    values.extend(out.clone());
                    let bytes = compose.render(&values, veredas_core::telegram::Endian::Little).map_err(|e| format!("`{}`: {e}", field.name))?;
                    String::from_utf8_lossy(&bytes).to_string()
                }
                // An earlier field of this list first, then the context.
                Source::From(name) => match out.get(name).or_else(|| ctx.fields.get(name)) {
                    Some(v) => v.clone(),
                    None => return Err(format!("`{}`: nothing named `{name}` to read from", field.name)),
                },
                Source::Register(name) => ctx.registers.get(name).copied().unwrap_or(0).to_string(),
                Source::Now(format) => {
                    let mut s = String::new();
                    use std::fmt::Write as _;
                    let _ = write!(s, "{}", Local::now().format(format));
                    s
                }
                Source::Decide(d) => decide(d, ctx).ok_or_else(|| format!("`{}`: no decision", field.name))?,
            };
            out.insert(field.name.clone(), value);
        }
        Ok(out)
    }
}

/// Decide a value: the sticky memory first (when it is still not blocked), then the first rule
/// that matches, then the spread over what is not blocked, then the reject value.
fn decide(d: &mut CompiledDecide, ctx: &mut Context<'_>) -> Option<String> {
    let blocked = |v: &str| v.parse::<u64>().ok().is_some_and(|n| ctx.blocked.contains(&n));
    let key = d.sticky.as_ref().and_then(|k| ctx.fields.get(k).cloned());
    if let Some(k) = &key
        && let Some(v) = d.remembered.get(k)
        && !blocked(v)
    {
        return Some(v.clone());
    }
    let mut decided = None;
    for (field, re, value) in &d.rules {
        let have = ctx.fields.get(field).map(|v| v.trim().to_string());
        let hit = match re {
            None => have.as_deref().is_none_or(str::is_empty),
            Some(re) => have.as_deref().is_some_and(|v| re.is_match(v)),
        };
        if hit && !blocked(value) {
            decided = Some(value.clone());
            break;
        }
    }
    if decided.is_none() {
        let eligible: Vec<&(String, f64)> = d.spread.iter().filter(|(v, w)| *w > 0.0 && !blocked(v)).collect();
        let total: f64 = eligible.iter().map(|(_, w)| w).sum();
        if total > 0.0 {
            let mut point = ctx.rng.uniform(0.0, total, 6);
            for (v, w) in &eligible {
                if point <= *w {
                    decided = Some(v.clone());
                    break;
                }
                point -= w;
            }
            if decided.is_none() {
                decided = eligible.last().map(|(v, _)| v.clone());
            }
        }
    }
    let decided = decided.or_else(|| d.reject.clone())?;
    if let Some(k) = key
        && d.remembered.insert(k.clone(), decided.clone()).is_none()
    {
        d.order.push_back(k);
        while d.order.len() > STICKY {
            if let Some(old) = d.order.pop_front() {
                d.remembered.remove(&old);
            }
        }
    }
    Some(decided)
}

// ---------------------------------------------------------------------------------------------
// The rules, compiled
// ---------------------------------------------------------------------------------------------

/// A compiled set of destinations.
#[derive(Clone, Debug)]
pub struct Destinations {
    pub list: Vec<u64>,
    pub bits: BTreeMap<String, u32>,
    pub resting: u32,
    pub initial: BTreeMap<u64, u32>,
    /// `(bit mask, fields forced, requeue)`, in precedence order.
    pub blocking: Vec<(String, u32, BTreeMap<String, String>, bool)>,
}

impl Destinations {
    /// Whether a word refuses an item: the first blocking bit set in it.
    pub fn blocking_of(&self, word: u32) -> Option<&(String, u32, BTreeMap<String, String>, bool)> {
        self.blocking.iter().find(|(_, mask, _, _)| word & mask != 0)
    }

    /// The names of the bits set in a word.
    pub fn names(&self, word: u32) -> Vec<String> {
        self.bits.iter().filter(|(_, m)| word & **m != 0).map(|(n, _)| n.clone()).collect()
    }
}

#[derive(Clone, Debug)]
pub struct Items {
    pub message: String,
    pub interval_ms: u64,
    pub key: String,
    pub gate: Option<String>,
    pub lap_ms: u64,
    pub fields: Fields,
    pub faults: Vec<(String, u8, BTreeMap<String, String>)>,
    pub reply: Option<Reply>,
    pub outcome: Option<Outcome>,
}

#[derive(Clone, Debug)]
pub struct Reply {
    pub message: String,
    pub key: String,
    pub destination: Option<String>,
    pub when: Vec<(String, String, Action)>,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub message: String,
    pub after_ms: u64,
    pub fields: Fields,
    pub mix: Vec<(String, f64, BTreeMap<String, String>, bool)>,
}

#[derive(Clone, Debug)]
pub struct Answer {
    pub on: String,
    pub send: String,
    pub after_ms: u64,
    pub fields: Fields,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub on: String,
    pub field: String,
    pub when: Vec<(String, BTreeMap<String, i64>)>,
}

#[derive(Clone, Debug)]
pub struct Cyclic {
    pub message: String,
    pub every_ms: u64,
    pub per_destination: bool,
    pub fields: Fields,
}

/// A loaded, validated controller file.
#[derive(Clone, Debug)]
pub struct Rules {
    pub seed: Option<u64>,
    pub registers: BTreeMap<String, i64>,
    pub destinations: Option<Destinations>,
    pub items: Option<Items>,
    pub answers: Vec<Answer>,
    pub commands: Vec<Command>,
    pub cyclic: Vec<Cyclic>,
}

impl Rules {
    /// The messages this controller sends, by name.
    pub fn sends(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(i) = &self.items {
            out.push(i.message.clone());
            if let Some(o) = &i.outcome {
                out.push(o.message.clone());
            }
        }
        out.extend(self.answers.iter().map(|a| a.send.clone()));
        out.extend(self.cyclic.iter().map(|c| c.message.clone()));
        out
    }

    /// The messages this controller reacts to, by name.
    pub fn receives(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(r) = self.items.as_ref().and_then(|i| i.reply.as_ref()) {
            out.push(r.message.clone());
        }
        out.extend(self.answers.iter().map(|a| a.on.clone()));
        out.extend(self.commands.iter().map(|c| c.on.clone()));
        out
    }
}

fn text_map(m: &BTreeMap<String, Literal>) -> BTreeMap<String, String> {
    m.iter().map(|(k, v)| (k.clone(), v.text())).collect()
}

/// A message's name checked against the catalog, and the names its body needs.
fn message<'a>(place: &str, catalog: &'a Catalog, name: &str) -> Result<&'a veredas_core::telegram::Message, RulesError> {
    catalog.by_name(name).ok_or_else(|| invalid(format!("{place}: the catalog has no message `{name}`")))
}

/// Read and validate a controller file against the catalog its messages come from.
pub fn load(text: &str, catalog: &Catalog) -> Result<Rules, RulesError> {
    let header: Header = toml::from_str(text).map_err(|e| RulesError::NotToml(e.to_string()))?;
    match header.format.as_deref() {
        Some(FORMAT) => {}
        Some(other) => return Err(RulesError::WrongFormat(other.to_string())),
        None => return Err(invalid("no `format`: a controller file states `format` and `version`")),
    }
    match header.version {
        Some(VERSION) => {}
        Some(v) => return Err(RulesError::UnknownVersion(v)),
        None => return Err(invalid("no `version`: a controller file states `format` and `version`")),
    }
    let file: FileV1 = toml::from_str(text).map_err(|e| RulesError::NotToml(e.to_string()))?;
    for name in file.registers.keys() {
        if name.trim().is_empty() {
            return Err(invalid("registers: a register has no name"));
        }
    }
    let registers = file.registers;

    let destinations = match file.destinations {
        None => None,
        Some(d) => {
            if d.list.is_empty() {
                return Err(invalid("destinations.list is empty"));
            }
            let mut seen = Vec::new();
            for n in &d.list {
                if seen.contains(n) {
                    return Err(invalid(format!("destinations.list: {n} is listed twice")));
                }
                seen.push(*n);
            }
            let mut masks: Vec<u32> = Vec::new();
            for (name, mask) in &d.bits {
                if name.trim().is_empty() {
                    return Err(invalid("destinations.bits: a bit has no name"));
                }
                if *mask == 0 || mask.count_ones() != 1 {
                    return Err(invalid(format!("destinations.bits.{name}: {mask} is not one bit")));
                }
                if masks.contains(mask) {
                    return Err(invalid(format!("destinations.bits.{name}: bit {mask} is named twice")));
                }
                masks.push(*mask);
            }
            let bit = |place: &str, n: &str| -> Result<u32, RulesError> {
                d.bits.get(n).copied().ok_or_else(|| invalid(format!("{place}: no bit named `{n}`")))
            };
            let mut resting = 0;
            for n in &d.resting {
                resting |= bit("destinations.resting", n)?;
            }
            let mut initial = BTreeMap::new();
            for (k, names) in &d.initial {
                let dest: u64 = k.trim().parse().map_err(|_| invalid(format!("destinations.initial: `{k}` is not a destination number")))?;
                if !d.list.contains(&dest) {
                    return Err(invalid(format!("destinations.initial: {dest} is not in the list")));
                }
                let mut word = 0;
                for n in names {
                    word |= bit(&format!("destinations.initial.{k}"), n)?;
                }
                initial.insert(dest, word);
            }
            let mut blocking = Vec::new();
            for (i, b) in d.blocking.iter().enumerate() {
                let mask = bit(&format!("destinations.blocking[{i}]"), &b.bit)?;
                if blocking.iter().any(|(_, m, _, _)| *m == mask) {
                    return Err(invalid(format!("destinations.blocking[{i}]: `{}` blocks twice", b.bit)));
                }
                blocking.push((b.bit.clone(), mask, text_map(&b.set), b.requeue));
            }
            Some(Destinations { list: d.list, bits: d.bits, resting, initial, blocking })
        }
    };

    let items = match file.items {
        None => None,
        Some(i) => {
            let m = message("items.message", catalog, &i.message)?;
            if i.interval_ms == 0 {
                return Err(invalid("items.interval_ms is 0"));
            }
            if i.lap_ms == 0 {
                return Err(invalid("items.lap_ms is 0"));
            }
            let fields = Fields::compile("items", &i.fields, Place::Items, &registers, Some(&m.names()))?;
            if !fields.names().contains(&i.key) {
                return Err(invalid(format!("items.key: no field named `{}` is announced", i.key)));
            }
            if let Some(g) = &i.gate
                && !registers.contains_key(g)
            {
                return Err(invalid(format!("items.gate: no register `{g}`")));
            }
            let mut faults = Vec::new();
            for (j, f) in i.faults.iter().enumerate() {
                let place = format!("items.faults[{j}]");
                if f.name.trim().is_empty() {
                    return Err(invalid(format!("{place}: the fault has no name")));
                }
                if faults.iter().any(|(n, _, _): &(String, u8, _)| n == &f.name) {
                    return Err(invalid(format!("{place}: two faults are named `{}`", f.name)));
                }
                if f.percent > 100 {
                    return Err(invalid(format!("{place}.percent is {}, over 100", f.percent)));
                }
                for k in f.set.keys() {
                    if !fields.names().contains(k) {
                        return Err(invalid(format!("{place}.set: no announced field named `{k}`")));
                    }
                }
                faults.push((f.name.clone(), f.percent, text_map(&f.set)));
            }
            let reply = match &i.reply {
                None => None,
                Some(r) => {
                    let rm = message("items.reply.message", catalog, &r.message)?;
                    let key = r.key.clone().unwrap_or_else(|| i.key.clone());
                    let binary = rm.capture_spec.payload.is_none();
                    let has = |n: &str| rm.capture_spec.fields.iter().any(|f| f.name == n) || rm.capture_spec.repeat.iter().any(|f| f.name == n);
                    if binary && !has(&key) {
                        return Err(invalid(format!("items.reply.key: `{}` has no field named `{key}`", r.message)));
                    }
                    if let Some(dst) = &r.destination
                        && binary
                        && !has(dst)
                    {
                        return Err(invalid(format!("items.reply.destination: `{}` has no field named `{dst}`", r.message)));
                    }
                    if r.destination.is_none() && i.outcome.is_some() && destinations.is_some() {
                        return Err(invalid("items.reply.destination: an outcome by destination needs the reply's destination field".to_string()));
                    }
                    let when = r.when.iter().map(|w| (w.field.clone(), w.equals.text(), w.action)).collect();
                    Some(Reply { message: r.message.clone(), key, destination: r.destination.clone(), when })
                }
            };
            let outcome = match &i.outcome {
                None => None,
                Some(o) => {
                    if reply.is_none() {
                        return Err(invalid("items.outcome: an outcome needs a reply to follow".to_string()));
                    }
                    let om = message("items.outcome.message", catalog, &o.message)?;
                    let fields = Fields::compile("items.outcome", &o.fields, Place::Outcome, &registers, Some(&om.names()))?;
                    let mut mix = Vec::new();
                    for (j, e) in o.mix.iter().enumerate() {
                        let place = format!("items.outcome.mix[{j}]");
                        if e.name.trim().is_empty() {
                            return Err(invalid(format!("{place}: the outcome has no name")));
                        }
                        if mix.iter().any(|(n, _, _, _): &(String, f64, _, bool)| n == &e.name) {
                            return Err(invalid(format!("{place}: two outcomes are named `{}`", e.name)));
                        }
                        if !e.weight.is_finite() || e.weight < 0.0 {
                            return Err(invalid(format!("{place}.weight is not a finite, non-negative number")));
                        }
                        mix.push((e.name.clone(), e.weight, text_map(&e.set), e.requeue));
                    }
                    Some(Outcome { message: o.message.clone(), after_ms: o.after_ms, fields, mix })
                }
            };
            Some(Items {
                message: i.message.clone(),
                interval_ms: i.interval_ms,
                key: i.key.clone(),
                gate: i.gate.clone(),
                lap_ms: i.lap_ms,
                fields,
                faults,
                reply,
                outcome,
            })
        }
    };

    let mut answers = Vec::new();
    for (i, a) in file.answers.iter().enumerate() {
        let place = format!("answers[{i}]");
        message(&format!("{place}.on"), catalog, &a.on)?;
        let send = message(&format!("{place}.send"), catalog, &a.send)?;
        let fields = Fields::compile(&place, &a.fields, Place::Answer, &registers, Some(&send.names()))?;
        answers.push(Answer { on: a.on.clone(), send: a.send.clone(), after_ms: a.after_ms, fields });
    }

    let mut commands = Vec::new();
    for (i, c) in file.commands.iter().enumerate() {
        let place = format!("commands[{i}]");
        message(&format!("{place}.on"), catalog, &c.on)?;
        if c.field.trim().is_empty() {
            return Err(invalid(format!("{place}.field is empty")));
        }
        if c.when.is_empty() {
            return Err(invalid(format!("{place}.when is empty: a command sets something")));
        }
        let mut when = Vec::new();
        for (j, w) in c.when.iter().enumerate() {
            for k in w.set.keys() {
                if !registers.contains_key(k) {
                    return Err(invalid(format!("{place}.when[{j}].set: no register `{k}`")));
                }
            }
            when.push((w.equals.text(), w.set.clone()));
        }
        commands.push(Command { on: c.on.clone(), field: c.field.clone(), when });
    }

    let mut cyclic = Vec::new();
    for (i, c) in file.cyclic.iter().enumerate() {
        let place = format!("cyclic[{i}]");
        let m = message(&format!("{place}.message"), catalog, &c.message)?;
        if c.every_ms == 0 {
            return Err(invalid(format!("{place}.every_ms is 0")));
        }
        if c.per_destination && destinations.is_none() {
            return Err(invalid(format!("{place}: per_destination needs [destinations]")));
        }
        let fields = Fields::compile(&place, &c.fields, Place::Cyclic { per_destination: c.per_destination }, &registers, Some(&m.names()))?;
        cyclic.push(Cyclic { message: c.message.clone(), every_ms: c.every_ms, per_destination: c.per_destination, fields });
    }

    Ok(Rules { seed: file.seed, registers, destinations, items, answers, commands, cyclic })
}

// ---------------------------------------------------------------------------------------------
// The engine
// ---------------------------------------------------------------------------------------------

/// The first instant after `now` in the series `due`, `due + step`, `due + 2 step`...: the next
/// due time, scheduled from the last one so that a late tick never stretches the interval, and
/// a stall skips what it missed rather than bunching it.
fn step_past(due: u64, step: u64, now: u64) -> u64 {
    let step = step.max(1);
    let mut next = due.saturating_add(step);
    if next <= now {
        let missed = (now - next) / step + 1;
        next = next.saturating_add(missed.saturating_mul(step));
    }
    next
}

/// An item the controller is tracking.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Its announced fields, and its reply's once answered.
    pub fields: HashMap<String, String>,
    /// Its key's value.
    pub key: String,
    /// When it was announced (the controller's clock, ms).
    pub announced_ms: u64,
    pub answered_ms: Option<u64>,
    /// The destination it was sent to.
    pub destination: Option<u64>,
    /// How many times it was announced.
    pub laps: u32,
    /// The peer that answered it.
    pub peer: Option<PeerId>,
    /// How it ended: an outcome's name, a blocking bit, `parked`, `delivered`.
    pub ended: Option<String>,
}

impl Item {
    /// How long the answer took, once it came.
    pub fn reply_latency_ms(&self) -> Option<u64> {
        self.answered_ms.map(|a| a.saturating_sub(self.announced_ms))
    }
}

/// Something the controller wants on the wire.
#[derive(Clone, Debug, PartialEq)]
pub struct Outgoing {
    pub message: String,
    pub fields: HashMap<String, String>,
    /// One peer (an answer goes back where its question came from), or everyone.
    pub to: Option<PeerId>,
    /// A line for the monitor.
    pub note: String,
}

/// The controller's counters.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub announced: u64,
    pub answered: u64,
    pub requeued: u64,
    pub parked: u64,
    /// Answers for an item the controller is not waiting on.
    pub unknown_answers: u64,
    /// Answers naming a destination the controller does not have.
    pub invalid_destinations: u64,
    /// Items ended, by how: an outcome's name, `blocked:<bit>`, or `delivered`.
    pub ended: BTreeMap<String, u64>,
    pub answers_sent: u64,
    /// Answers refused for want of a decision or a value.
    pub answers_refused: u64,
    pub commands: u64,
    pub cyclic_sent: u64,
    /// Items announced with each fault.
    pub faults: BTreeMap<String, u64>,
}

/// What the controller reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ControllerStatus {
    pub registers: BTreeMap<String, i64>,
    /// Every destination's state word.
    pub destinations: BTreeMap<u64, u32>,
    /// Announced, awaiting a reply.
    pub awaiting: Vec<Item>,
    /// The age of the oldest unanswered item: the number that says the peer stopped replying.
    pub oldest_unanswered_ms: Option<u64>,
    /// Ended items, newest first, at most [`HISTORY`].
    pub history: Vec<Item>,
    pub stats: Stats,
    pub interval_ms: Option<u64>,
    pub faults: BTreeMap<String, u8>,
}

/// The simulated controller, pure and tick-driven.
pub struct Controller {
    rules: Rules,
    registers: BTreeMap<String, i64>,
    destinations: BTreeMap<u64, u32>,
    rng: Pcg32,
    now_ms: u64,
    next_item_ms: u64,
    awaiting: Vec<Item>,
    travelling: Vec<(Item, u64)>,
    requeued: VecDeque<(Item, u64)>,
    history: VecDeque<Item>,
    pending: Vec<(u64, Outgoing)>,
    next_cyclic: Vec<u64>,
    stats: Stats,
}

impl Controller {
    /// A controller running `rules`; `seed` (else the file's, else the clock) seeds its draws.
    pub fn new(rules: Rules, seed: Option<u64>) -> Controller {
        let seed = seed.or(rules.seed);
        let registers = rules.registers.clone();
        let destinations = rules
            .destinations
            .as_ref()
            .map(|d| d.list.iter().map(|n| (*n, d.initial.get(n).copied().unwrap_or(d.resting))).collect())
            .unwrap_or_default();
        let next_cyclic = rules.cyclic.iter().map(|c| c.every_ms).collect();
        Controller {
            registers,
            destinations,
            rng: match seed {
                Some(s) => Pcg32::new(s, 9),
                None => Pcg32::from_entropy(9),
            },
            now_ms: 0,
            next_item_ms: 0,
            awaiting: Vec::new(),
            travelling: Vec::new(),
            requeued: VecDeque::new(),
            history: VecDeque::new(),
            pending: Vec::new(),
            next_cyclic,
            stats: Stats::default(),
            rules,
        }
    }

    pub fn rules(&self) -> &Rules {
        &self.rules
    }

    pub fn now_ms(&self) -> u64 {
        self.now_ms
    }

    /// The destinations refusing an item right now.
    fn blocked(&self) -> Vec<u64> {
        match &self.rules.destinations {
            Some(d) => self.destinations.iter().filter(|(_, w)| d.blocking_of(**w).is_some()).map(|(n, _)| *n).collect(),
            None => Vec::new(),
        }
    }

    fn running(&self) -> bool {
        match self.rules.items.as_ref().and_then(|i| i.gate.as_ref()) {
            Some(g) => self.registers.get(g).copied().unwrap_or(0) != 0,
            None => true,
        }
    }

    /// Advance the clock by `elapsed_ms` and produce what is due. Items are announced only while
    /// `peers` is non-zero (an item announced into nothing would only time out) and the gate
    /// register, if any, is non-zero.
    pub fn tick(&mut self, elapsed_ms: u64, peers: usize) -> Vec<Outgoing> {
        self.now_ms += elapsed_ms;
        let mut out = Vec::new();

        // Items whose lap has come round, held like a new item while nobody is there to hear
        // them or the gate is closed.
        let announcing = peers > 0 && self.running();
        while announcing && let Some((_, due)) = self.requeued.front() {
            if *due > self.now_ms {
                break;
            }
            let (item, _) = self.requeued.pop_front().unwrap_or_else(|| unreachable!("front was there"));
            if let Some(o) = self.announce(item) {
                out.push(o);
            }
        }
        if self.rules.items.is_some() && announcing && self.now_ms >= self.next_item_ms {
            let interval = self.rules.items.as_ref().map(|i| i.interval_ms).unwrap_or(1).max(1);
            // Due an interval after the last was due, never after the tick that served it, so
            // a late tick does not stretch the interval; a stall skips what it missed.
            self.next_item_ms = step_past(self.next_item_ms, interval, self.now_ms);
            if let Some(o) = self.announce_new() {
                out.push(o);
            }
        }
        // Outcomes that are due.
        let due: Vec<(Item, u64)> = std::mem::take(&mut self.travelling);
        for (item, at) in due {
            if at > self.now_ms {
                self.travelling.push((item, at));
            } else if let Some(o) = self.report(item) {
                out.push(o);
            }
        }
        // Answers whose delay has passed.
        let pending = std::mem::take(&mut self.pending);
        for (at, o) in pending {
            if at > self.now_ms {
                self.pending.push((at, o));
            } else {
                self.stats.answers_sent += 1;
                out.push(o);
            }
        }
        // Cyclic messages.
        for i in 0..self.rules.cyclic.len() {
            if self.now_ms < self.next_cyclic[i] {
                continue;
            }
            self.next_cyclic[i] = step_past(self.next_cyclic[i], self.rules.cyclic[i].every_ms, self.now_ms);
            out.extend(self.cyclic(i));
        }
        out
    }

    fn cyclic(&mut self, i: usize) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let per_destination = self.rules.cyclic[i].per_destination;
        let contexts: Vec<HashMap<String, String>> = if per_destination {
            self.destinations
                .iter()
                .map(|(n, w)| HashMap::from([("destination".to_string(), n.to_string()), ("state".to_string(), w.to_string())]))
                .collect()
        } else {
            vec![HashMap::new()]
        };
        let blocked = self.blocked();
        let message = self.rules.cyclic[i].message.clone();
        for ctx_fields in contexts {
            let mut ctx = Context { fields: &ctx_fields, registers: &self.registers, blocked: &blocked, rng: &mut self.rng };
            match self.rules.cyclic[i].fields.render(&mut ctx) {
                Ok(fields) => {
                    self.stats.cyclic_sent += 1;
                    let note = match ctx_fields.get("destination") {
                        Some(d) => format!("{message} for destination {d}"),
                        None => message.clone(),
                    };
                    out.push(Outgoing { message: message.clone(), fields, to: None, note });
                }
                Err(e) => out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{message} not sent: {e}") }),
            }
        }
        out
    }

    fn announce_new(&mut self) -> Option<Outgoing> {
        let items = self.rules.items.as_mut()?;
        let empty = HashMap::new();
        let blocked: Vec<u64> = Vec::new();
        let mut ctx = Context { fields: &empty, registers: &self.registers, blocked: &blocked, rng: &mut self.rng };
        let mut fields = match items.fields.render(&mut ctx) {
            Ok(f) => f,
            Err(e) => {
                return Some(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{} not announced: {e}", items.message) });
            }
        };
        let mut faulted = Vec::new();
        for (name, percent, set) in &items.faults {
            if *percent > 0 && self.rng.int_in(1, 100) <= *percent as i64 {
                for (k, v) in set {
                    fields.insert(k.clone(), v.clone());
                }
                faulted.push(name.clone());
            }
        }
        for f in &faulted {
            *self.stats.faults.entry(f.clone()).or_default() += 1;
        }
        let key = fields.get(&items.key).cloned().unwrap_or_default();
        let item = Item { fields, key, announced_ms: self.now_ms, answered_ms: None, destination: None, laps: 1, peer: None, ended: None };
        let _ = faulted;
        self.announce(item)
    }

    /// Announce an item (new, or again).
    fn announce(&mut self, mut item: Item) -> Option<Outgoing> {
        let items = self.rules.items.as_ref()?;
        item.announced_ms = self.now_ms;
        item.answered_ms = None;
        item.destination = None;
        item.peer = None;
        let note = format!(
            "{} {}{}",
            items.message,
            item.key,
            if item.laps > 1 { format!(" (lap {})", item.laps) } else { String::new() }
        );
        self.stats.announced += 1;
        let out = Outgoing { message: items.message.clone(), fields: item.fields.clone(), to: None, note };
        self.awaiting.push(item);
        Some(out)
    }

    fn requeue(&mut self, mut item: Item) {
        let lap = self.rules.items.as_ref().map(|i| i.lap_ms).unwrap_or(1);
        item.laps += 1;
        self.stats.requeued += 1;
        self.requeued.push_back((item, self.now_ms + lap));
    }

    fn end(&mut self, mut item: Item, how: &str) {
        item.ended = Some(how.to_string());
        *self.stats.ended.entry(how.to_string()).or_default() += 1;
        self.history.push_front(item);
        while self.history.len() > HISTORY {
            self.history.pop_back();
        }
    }

    /// An item's travel ended: report the outcome, forced by the destination's state or drawn
    /// from the mix.
    fn report(&mut self, item: Item) -> Option<Outgoing> {
        let outcome = self.rules.items.as_ref()?.outcome.clone()?;
        let forced = item.destination.and_then(|d| {
            let word = *self.destinations.get(&d)?;
            self.rules.destinations.as_ref()?.blocking_of(word).cloned()
        });
        let (how, set, requeue) = match forced {
            Some((bit, _, set, requeue)) => (format!("blocked:{bit}"), set, requeue),
            None => {
                let enabled: Vec<&(String, f64, BTreeMap<String, String>, bool)> = outcome.mix.iter().filter(|(_, w, _, _)| *w > 0.0).collect();
                let total: f64 = enabled.iter().map(|(_, w, _, _)| w).sum();
                if total <= 0.0 {
                    ("delivered".to_string(), BTreeMap::new(), false)
                } else {
                    let mut point = self.rng.uniform(0.0, total, 6);
                    let mut chosen = enabled[enabled.len() - 1];
                    for e in &enabled {
                        if point <= e.1 {
                            chosen = e;
                            break;
                        }
                        point -= e.1;
                    }
                    (chosen.0.clone(), chosen.2.clone(), chosen.3)
                }
            }
        };
        let mut ctx_fields = item.fields.clone();
        for (k, v) in &set {
            ctx_fields.insert(k.clone(), v.clone());
        }
        let blocked = self.blocked();
        let mut fields_spec = outcome.fields.clone();
        let mut ctx = Context { fields: &ctx_fields, registers: &self.registers, blocked: &blocked, rng: &mut self.rng };
        let rendered = fields_spec.render(&mut ctx);
        // The outcome's own fields are kept in the rules (counters, cycles).
        if let Some(i) = self.rules.items.as_mut()
            && let Some(o) = i.outcome.as_mut()
        {
            o.fields = fields_spec;
        }
        let mut fields = match rendered {
            Ok(f) => f,
            Err(e) => {
                let note = format!("{} for {} not sent: {e}", outcome.message, item.key);
                self.end(item, "unreported");
                return Some(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note });
            }
        };
        // What the outcome forces or draws overrides what the fields rendered.
        for (k, v) in &set {
            fields.insert(k.clone(), v.clone());
        }
        let note = format!(
            "{} for {} at {}: {how}",
            outcome.message,
            item.key,
            item.destination.map(|d| d.to_string()).unwrap_or_else(|| "-".into())
        );
        let to = item.peer;
        let message = outcome.message.clone();
        if requeue {
            let mut again = item.clone();
            again.ended = Some(how.clone());
            self.history.push_front(again);
            while self.history.len() > HISTORY {
                self.history.pop_back();
            }
            *self.stats.ended.entry(how.clone()).or_default() += 1;
            self.requeue(item);
        } else {
            self.end(item, &how);
        }
        Some(Outgoing { message, fields, to, note })
    }

    /// A decoded message arrived from `peer`: the controller's answers, the reply it awaited,
    /// the command it obeys. Returns what it sends now or later, and notes for the monitor.
    pub fn on_message(&mut self, name: &str, fields: &HashMap<String, String>, peer: Option<PeerId>) -> Vec<Outgoing> {
        let mut out = Vec::new();
        // The reply to an item.
        let reply = self.rules.items.as_ref().and_then(|i| i.reply.clone()).filter(|r| r.message == name);
        if let Some(reply) = reply {
            let key = fields.get(&reply.key).map(|v| v.trim().to_string()).unwrap_or_default();
            match self.awaiting.iter().position(|it| it.key == key) {
                None => {
                    self.stats.unknown_answers += 1;
                    out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{name} for {key}, which is not awaited") });
                }
                Some(i) => {
                    let mut item = self.awaiting.remove(i);
                    // The reply's fields join the item's, trimmed as the controller reads them.
                    for (k, v) in fields {
                        item.fields.insert(k.clone(), v.trim().to_string());
                    }
                    item.answered_ms = Some(self.now_ms);
                    item.peer = peer;
                    self.stats.answered += 1;
                    let action = reply
                        .when
                        .iter()
                        .find(|(f, eq, _)| fields.get(f).map(|v| v.trim() == eq.as_str()).unwrap_or(false))
                        .map(|(_, _, a)| *a)
                        .unwrap_or(Action::Deliver);
                    let latency = item.reply_latency_ms().unwrap_or(0);
                    match action {
                        Action::Requeue => {
                            out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{key}: no destination yet, round again") });
                            self.requeue(item);
                        }
                        Action::Park => {
                            self.stats.parked += 1;
                            out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{key}: parked") });
                            self.end(item, "parked");
                        }
                        Action::Deliver => {
                            let dest = reply.destination.as_ref().and_then(|d| fields.get(d)).and_then(|v| v.trim().parse::<u64>().ok());
                            item.destination = dest;
                            let known = match (&self.rules.destinations, dest) {
                                (Some(d), Some(n)) => d.list.contains(&n),
                                (Some(_), None) => false,
                                (None, _) => true,
                            };
                            if !known {
                                self.stats.invalid_destinations += 1;
                            }
                            out.push(Outgoing {
                                message: String::new(),
                                fields: HashMap::new(),
                                to: None,
                                note: format!(
                                    "{key} to {}{} ({latency} ms)",
                                    dest.map(|d| d.to_string()).unwrap_or_else(|| "-".into()),
                                    if known { "" } else { ", which is unknown" }
                                ),
                            });
                            match self.rules.items.as_ref().and_then(|i| i.outcome.as_ref()) {
                                Some(o) => self.travelling.push((item, self.now_ms + o.after_ms)),
                                None => self.end(item, "delivered"),
                            }
                        }
                    }
                }
            }
        }
        // Answers.
        let blocked = self.blocked();
        for i in 0..self.rules.answers.len() {
            if self.rules.answers[i].on != name {
                continue;
            }
            let mut ctx = Context { fields, registers: &self.registers, blocked: &blocked, rng: &mut self.rng };
            match self.rules.answers[i].fields.render(&mut ctx) {
                Ok(values) => {
                    let a = &self.rules.answers[i];
                    let o = Outgoing { message: a.send.clone(), fields: values, to: peer, note: format!("{} to {name}", a.send) };
                    if a.after_ms == 0 {
                        self.stats.answers_sent += 1;
                        out.push(o);
                    } else {
                        self.pending.push((self.now_ms + a.after_ms, o));
                    }
                }
                Err(e) => {
                    self.stats.answers_refused += 1;
                    out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{} to {name} not sent: {e}", self.rules.answers[i].send) });
                }
            }
        }
        // Commands.
        for c in &self.rules.commands {
            if c.on != name {
                continue;
            }
            let value = fields.get(&c.field).map(|v| v.trim().to_string()).unwrap_or_default();
            match c.when.iter().find(|(eq, _)| *eq == value) {
                Some((_, set)) => {
                    for (k, v) in set {
                        self.registers.insert(k.clone(), *v);
                    }
                    self.stats.commands += 1;
                    let what: Vec<String> = set.iter().map(|(k, v)| format!("{k} = {v}")).collect();
                    out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{name} {value}: {}", what.join(", ")) });
                }
                None => out.push(Outgoing { message: String::new(), fields: HashMap::new(), to: None, note: format!("{name} {value}: no such command") }),
            }
        }
        out
    }

    // ---- the knobs ----

    /// Announce an item now, whatever the interval; nothing without items.
    pub fn announce_now(&mut self) -> Option<Outgoing> {
        self.rules.items.as_ref()?;
        self.announce_new()
    }

    /// The announcement interval, live.
    pub fn set_interval(&mut self, ms: u64) -> Result<(), RulesError> {
        let items = self.rules.items.as_mut().ok_or_else(|| invalid("no items to announce"))?;
        if ms == 0 {
            return Err(invalid("the interval is 0"));
        }
        items.interval_ms = ms;
        self.next_item_ms = self.next_item_ms.min(self.now_ms + ms);
        Ok(())
    }

    /// A fault's share of items, live.
    pub fn set_fault(&mut self, name: &str, percent: u8) -> Result<(), RulesError> {
        if percent > 100 {
            return Err(invalid(format!("{percent} is over 100")));
        }
        let items = self.rules.items.as_mut().ok_or_else(|| invalid("no items to announce"))?;
        let fault = items.faults.iter_mut().find(|(n, _, _)| n == name).ok_or_else(|| invalid(format!("no fault named `{name}`")))?;
        fault.1 = percent;
        Ok(())
    }

    /// A destination's whole state word.
    pub fn set_destination(&mut self, destination: u64, word: u32) -> Result<(), RulesError> {
        if !self.destinations.contains_key(&destination) {
            return Err(invalid(format!("no destination {destination}")));
        }
        self.destinations.insert(destination, word);
        Ok(())
    }

    /// One named bit of a destination's word, set or cleared.
    pub fn set_bit(&mut self, destination: u64, bit: &str, on: bool) -> Result<(), RulesError> {
        let mask = self
            .rules
            .destinations
            .as_ref()
            .and_then(|d| d.bits.get(bit).copied())
            .ok_or_else(|| invalid(format!("no bit named `{bit}`")))?;
        let word = self.destinations.get_mut(&destination).ok_or_else(|| invalid(format!("no destination {destination}")))?;
        if on {
            *word |= mask;
        } else {
            *word &= !mask;
        }
        Ok(())
    }

    /// One named bit on every destination.
    pub fn set_bit_everywhere(&mut self, bit: &str, on: bool) -> Result<(), RulesError> {
        let list: Vec<u64> = self.destinations.keys().copied().collect();
        for d in list {
            self.set_bit(d, bit, on)?;
        }
        Ok(())
    }

    /// A register's value.
    pub fn set_register(&mut self, name: &str, value: i64) -> Result<(), RulesError> {
        if !self.registers.contains_key(name) {
            return Err(invalid(format!("no register `{name}`")));
        }
        self.registers.insert(name.to_string(), value);
        Ok(())
    }

    pub fn status(&self) -> ControllerStatus {
        ControllerStatus {
            registers: self.registers.clone(),
            destinations: self.destinations.clone(),
            awaiting: self.awaiting.clone(),
            oldest_unanswered_ms: self.awaiting.iter().map(|i| self.now_ms.saturating_sub(i.announced_ms)).max(),
            history: self.history.iter().cloned().collect(),
            stats: self.stats.clone(),
            interval_ms: self.rules.items.as_ref().map(|i| i.interval_ms),
            faults: self.rules.items.as_ref().map(|i| i.faults.iter().map(|(n, p, _)| (n.clone(), *p)).collect()).unwrap_or_default(),
        }
    }
}
