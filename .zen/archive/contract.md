# Archived Contract Items

Retired clauses, preserved — never deleted (the audit trace, C-023). Each carries the date it left the live contract.

<!-- archived 2026-11-05: shelved -->
## C-011: The GENIBELT catalog: every typed message decoded and encoded per the specification
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md; the GENIBELT spec §3.3, §4, docs/questions.md Q6)
- **Description:** Every GENIBELT message the specification defines is a typed `Message` with its id, its exact MSG LENGTH and field layout, decoded from and encoded to a segment body; the variable-length messages carry their tails; a body of the wrong length, or longer than the 508 bytes a 512-byte segment holds beside its header, is an error, never a panic; an unknown id is kept verbatim; fixed-width text fields survive (an ASCIIZ field reads back without trailing spaces), an ASCIIZ field always ends in NUL, and neither it nor a UTF-16 field ever ends in half a character; outcome codes and every named bin-state bit read as words, and the bin-state bits say whether a chute takes parcels (whether the TRISTATE bits do is `docs/questions.md` Q7).
- **Verification:** automated → crates/veredas-core/tests/telegram_genibelt.rs | Veredas 1's tests: every declared size with a round trip, the destination's fields, the scanner request's physicals and no-read, outcome words, bin-state bits, fixed-width text, wrong lengths, unknown ids, variable tails; plus golden bodies built by hand from the specification's offset tables for every message and the ISPC/FSG block (decode to the fields, encode back to the bytes, same-width fields distinct), the outcome, flag and bin-state tables by number, the output name's NUL, the four high bin-state bits, the body limit at 508/509, a surrogate pair at the block's edge
- **Evidence:** .zen/evidence/C-011.md
- **Status:** pending

<!-- archived 2026-11-05: shelved -->
## C-014: The sort plan: how a parcel becomes a destination
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md; Veredas 1's docs/sort-plan.md)
- **Description:** Routing rules, ordered and first match wins, decide by content (a plan of rules alone too) and their answer is sticky once actually sent, keyed on the plan's named field (a blank value is no identity, and a field key never meets an object-id key), until its chute stops accepting or cannot take the parcel: a table answer judged by the table it came from, a rule's own chute by the main table, trusted when unlisted there; otherwise a weighted spread draws among eligible destinations, each finite positive weight, however large, a share of the enabled sum, per infeed when the plan says so (an infeed written `01` is infeed `1`); constraints exclude a chute that cannot take the parcel in both tiers, a value at its limit admitted; a blocked chute leaves the spread until it recovers; nothing eligible reaches the reject destination, or reports no decision when there is none; a rule may send to a named table (`@name`) instead of a chute; a broken rule (bad pattern, unknown table, no such chute) is a warning that disables it alone; sticky memory is bounded, keeping the newest (an answer remembered again is the newest); a seeded plan replays its draws exactly; tier counters make a fallback visible.
- **Verification:** automated → crates/veredas-core/tests/telegram_sortplan.rs | Veredas 1's tests, plus a rules-only plan, first match among two, shares with disabled and blocked heavy rows and per infeed, `01` against `1`, fractional and non-finite weights, one valid rule among broken ones, a remembered table answer over its limit, the sticky memory's newest kept and its named key, every limit and its edge, the reject counter
- **Evidence:** .zen/evidence/C-014.md
- **Status:** pending

<!-- archived 2026-11-05: shelved -->
## C-015: The decision path: bounded calls, never an invented destination
- **Date:** 2026-11-05
- **Source:** user (docs/core-plan.md)
- **Description:** A `DecisionClient` posts JSON to `http://host:port` (`/decide`, `/events`, `/binstate`), one fresh connection per call, bounded by its timeout; any failure is a typed `DecisionError` (not http, bad port, connect, write, read, timeout, no headers, unreadable status, a non-2xx refusal, encode, decode), never a guessed destination, and an answer without a destination is an answer. Lifecycle events go through a bounded queue that drops the oldest when full, never blocking, and a failed batch goes back to the front in order.
- **Verification:** automated → crates/veredas-core/tests/telegram_decision.rs | over real loopback sockets against a stand-in service answering raw bytes: refused addresses, a path after the port, success and every refusal shape, the request each endpoint sends, an unreachable and a silent service, a connect that never completes and a reply dripping in (each within the bound, not merely before 2 s), the real exchange, a null destination, each request read whole by its Content-Length; the queue's overflow, requeue, and requeue onto a full queue
- **Evidence:** .zen/evidence/C-015.md
- **Status:** pending
