# 0057 - Clock rollback keeps the `TtlSortedCache` eviction horizon monotonic

Status: Implemented

## Context

`TtlSortedCache` stores entries in a `HashMap<K, Entry<K, V>>` and a deadline-ordered
`BTreeSet<Stamped<K>>`. Its sweep detaches the prefix strictly before one cutoff with
`split_off`, so ordinary cleanup costs `O(log n + removed)` and does not scan the map.

A deadline index only supports that prefix sweep while the cutoff does not move backwards. A
clock source that can step backwards (a test clock, a fake target clock, or a platform where
`Instant` is not fully monotonic) can make a deadline that an earlier reading had already passed
appear future again. Repeated backward steps can then leave memory occupied by entries that the
operational clock horizon had already classified as expired.

On ordinary native and browser targets, `crate::time::Instant` is `web_time::Instant`, which
re-exports `std::time::Instant` on native and is backed by `Performance.now()` on the web.
System wall-clock adjustments do not move those clocks backwards. The high-water mark is
defense for clock fakes and platforms or embeddings with weaker guarantees.

## Decision

Add a private `clock_horizon: Instant` field to `TtlSortedCache`.

- Mutable clock entry points call `observe_clock(now)`, which updates the field to the maximum
  seen reading and returns that saturated cutoff.
- `evict`, `retain`, insertion-driven sweeps, and displaced-entry classification use the
  saturated cutoff.
- New deadlines remain anchored to the raw `now` sample from the insertion operation. Saturating
  the deadline itself would shorten new TTLs by the clock step; only liveness and cleanup use
  the high-water mark.
- Shared read paths compute `Instant::now().max(clock_horizon)` so reads and cleanup agree. They
  cannot persist a new high-water mark through `&self`; the next mutating operation does.
- `cache_reset` resets the field to the new cache epoch.

No whole-map reconciliation runs on a clock step. The `BTreeSet` is ordered by the same deadline
values used for liveness, so a saturated cutoff preserves the prefix invariant. The common case
still performs one tree descent plus removal of the expired prefix.

## Testing

The private unit tests in `src/stores/ttl_sorted.rs` pin the behavior with exact instants:

- `clock_rollback_does_not_roll_back_the_eviction_horizon` places many deadlines between a
  stepped-back reading and the previous high reading, verifies that an explicit sweep at the
  stepped-back cutoff removes nothing, then verifies that `evict()` with the stored horizon
  removes the whole batch and restores an empty map/index.
- `clock_rollback_does_not_revive_an_entry_for_reads` verifies the shared read classification.

## Notes

- No public API or Cargo feature changes.
- Other TTL stores use map scans or LRU-specific structures and are outside this record.
