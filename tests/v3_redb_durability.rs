//! Crash-snapshot tests for `RedbCache`'s bounded durability policy.
//!
//! A real power loss cannot be staged in-process, so these tests copy the backing file while
//! the cache is still open. The copy is the durable state a crash would leave behind: redb's
//! live, non-durable pages are not part of a file that a fresh instance can open and see them.

#![cfg(feature = "redb_store")]

use cached::time::Duration;
use cached::{ConcurrentCached, RedbCache};
use tempfile::TempDir;

const NAME: &str = "redb-durability";

fn build(dir: &TempDir, durable: bool, flush_interval: Option<Duration>) -> RedbCache<u32, u32> {
    let mut builder = RedbCache::<u32, u32>::builder(NAME)
        .disk_dir(dir.path())
        .durable(durable);
    if let Some(interval) = flush_interval {
        builder = builder.flush_interval(interval);
    }
    builder.build().expect("build RedbCache")
}

/// Read `key` from a copy of the live database file, i.e. from the state that survives a
/// crash at this instant.
fn crash_snapshot(cache: &RedbCache<u32, u32>, key: u32) -> Option<u32> {
    let snapshot_dir = TempDir::new().expect("snapshot temp dir");
    let file_name = cache
        .disk_path()
        .file_name()
        .expect("cache path has a file name");
    std::fs::copy(cache.disk_path(), snapshot_dir.path().join(file_name))
        .expect("copy live redb file");

    let snapshot: RedbCache<u32, u32> = RedbCache::builder(NAME)
        .disk_dir(snapshot_dir.path())
        .durable(true)
        .build()
        .expect("open crash snapshot");
    snapshot.cache_get(&key).expect("read crash snapshot")
}

#[test]
fn durable_true_write_is_in_the_crash_snapshot_immediately() {
    let dir = TempDir::new().expect("temp dir");
    let cache = build(&dir, true, None);

    cache.cache_set(1, 10).expect("write");

    assert_eq!(
        crash_snapshot(&cache, 1),
        Some(10),
        "durable(true) must fsync before cache_set returns"
    );
}

#[test]
fn durable_false_does_not_sync_every_write() {
    let dir = TempDir::new().expect("temp dir");
    let cache = build(&dir, false, Some(Duration::from_secs(60)));

    cache.cache_set(1, 10).expect("write");

    assert_eq!(
        crash_snapshot(&cache, 1),
        None,
        "durable(false) must not pay an fsync per write"
    );
}

#[test]
fn durable_false_flushes_within_the_configured_interval() {
    let dir = TempDir::new().expect("temp dir");
    let cache = build(&dir, false, Some(Duration::from_millis(25)));

    cache.cache_set(1, 10).expect("write");
    std::thread::sleep(Duration::from_millis(250));

    assert_eq!(
        crash_snapshot(&cache, 1),
        Some(10),
        "the background flusher must persist pending writes without an explicit flush"
    );
}

#[test]
fn durable_false_keeps_older_flushed_writes_when_newer_ones_are_pending() {
    let dir = TempDir::new().expect("temp dir");
    let cache = build(&dir, false, Some(Duration::from_millis(100)));

    cache.cache_set(1, 10).expect("write durable point");
    std::thread::sleep(Duration::from_millis(350));
    assert_eq!(
        crash_snapshot(&cache, 1),
        Some(10),
        "the first write must be durable after an interval tick"
    );

    cache.cache_set(2, 20).expect("write pending value");
    // A crash now may lose key 2, but it must not take key 1 with it. The bounded window is
    // the point: recent writes can be lost, the whole cache cannot reset to an old state.
    assert_eq!(
        crash_snapshot(&cache, 1),
        Some(10),
        "a newer pending write must not discard the previous durable state"
    );
}
