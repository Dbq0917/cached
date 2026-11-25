# 0058 - `RedbCache` non-durable writes get a bounded crash-loss window

Status: Implemented

## Context

`RedbCacheBuilder::durable` exposed two redb modes:

- `durable(true)` commits with `Durability::Immediate`, so every mutation is fsync'd before it
  returns.
- `durable(false)` commits with `Durability::None`. redb keeps such a commit in the live database
  state but does not make it part of the durable root until a later `Durability::Immediate`
  commit (or a graceful close).

The second mode was documented as an explicit-flush workflow, but nothing forced a flush. A
process that only wrote and never called `flush()` could lose an unbounded history on power
loss: a cache described as a disk cache, and documented as persisting entries, could come back
at an arbitrarily old state. The default was `durable(true)`, which avoided this at the cost of
an fsync per write. Flipping the default back to `false` without another mechanism would only
restore the unbounded-loss failure.

## Decision

Keep `durable(false)` as the lower-write-cost mode, but make it bounded:

- Every non-durable mutation still commits with `Durability::None`.
- When a cache is built with `durable(false)`, it starts one background flusher thread. The
  thread checks for pending mutations at most once per flush interval and issues one
  `Durability::Immediate` empty commit to promote all of them.
- The default durability is `false`, matching the "off is not fsync-per-write" requirement. The
  default flush interval is 500 ms.
- `RedbCacheBuilder::flush_interval(Duration)` controls the bound. A zero interval is rejected
  at build time; `durable(true)` is the way to request an fsync per write.
- `RedbCache::flush()` / `async_flush()` still force an immediate durable commit, and the
  flusher also performs a final pending flush when the cache is dropped.

The pending counter is advanced when a write transaction begins and the flushed counter only
after a successful durable commit. A write that races the flusher is therefore either included
in that flush or remains pending for the next one; it cannot be marked clean without being
persisted.

This is a bounded-loss guarantee, not the same guarantee as `durable(true)`: a crash can lose
the writes since the last completed flush, up to roughly one interval (plus the time of an
in-flight write). The point is that the loss window no longer grows without limit and does not
require every write to pay for an fsync.

## Testing

`tests/v3_redb_durability.rs` treats a copy of the live redb file as the state a crash would
leave behind:

- `durable_true_write_is_in_the_crash_snapshot_immediately`
- `durable_false_does_not_sync_every_write`
- `durable_false_flushes_within_the_configured_interval`
- `durable_false_keeps_older_flushed_writes_when_newer_ones_are_pending`

Private unit tests also pin the default policy, the absence of a flusher under `durable(true)`,
and rejection of a zero flush interval.

## Notes

- No Cargo feature change. The flusher uses `std::thread`, which is already part of every
  `redb_store` build.
- `durable(false)` previously meant "pending until an explicit durable commit"; it now means
  "pending for at most one flush interval". This is a behavior change, recorded in
  `CHANGELOG.md`.
