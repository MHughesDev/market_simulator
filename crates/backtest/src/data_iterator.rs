// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

//! Multi-stream, time-ordered data iterator for replaying historical data.

use std::{collections::BinaryHeap, fmt::Debug};

use ahash::AHashMap;
use nautilus_core::UnixNanos;
use nautilus_model::data::{Data, HasTsInit};

#[cfg(feature = "defi")]
use crate::defi::replay::replay_position;

// TODO: block_number/transaction_index/log_index/phase are DeFi-only (zero for all other data,
// even in non-DeFi builds); they exist to order same-block DeFi events in canonical chain order.
// This leaks DeFi-specific shape into a general key, so it could be cfg-gated or moved behind an
// opaque secondary key later (non-breaking, no correctness or perf cost).
#[derive(Debug, Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
struct ReplayKey {
    ts: UnixNanos,
    block_number: u64,
    transaction_index: u32,
    log_index: u32,
    phase: u8,
}

fn replay_key(data: &Data) -> ReplayKey {
    match data {
        #[cfg(feature = "defi")]
        Data::Defi(defi) => {
            let (block_number, transaction_index, log_index, phase) = replay_position(defi);
            ReplayKey {
                ts: defi.ts_init(),
                block_number,
                transaction_index,
                log_index,
                phase,
            }
        }
        _ => ReplayKey {
            ts: data.ts_init(),
            block_number: 0,
            transaction_index: 0,
            log_index: 0,
            phase: 0,
        },
    }
}

/// Internal convenience struct to keep heap entries ordered by replay key and priority.
#[derive(Debug, Eq, PartialEq)]
struct HeapEntry {
    key: ReplayKey,
    priority: i32,
    index: usize,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // min-heap on replay key, then priority sign (+/-) then index
        self.key
            .cmp(&other.key)
            .then_with(|| self.priority.cmp(&other.priority))
            .then_with(|| self.index.cmp(&other.index))
            .reverse() // BinaryHeap is max by default -> reverse for min behaviour
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A lazily-pulled source of pre-chronological-order [`Data`] chunks.
///
/// Implementations hand the iterator one chunk of data at a time on demand,
/// allowing a backtest to stream over time series far larger than can be
/// comfortably materialized in memory as a single `Vec<Data>` (mirroring the
/// proven `Generator[list[Data], None, None]` contract from the legacy Cython
/// engine's `add_data_iterator`/`init_data`).
///
/// Each returned chunk does not need to be pre-sorted — the iterator sorts it
/// by replay key before merging, exactly as it does for eagerly materialized
/// streams added via [`BacktestDataIterator::add_data`]. However, chunks
/// themselves must be supplied in non-decreasing chronological sequence (the
/// last element of one chunk must not be later than the first element of the
/// next): the iterator replaces one chunk with the next rather than re-merging
/// against already-replayed data, mirroring the legacy Cython engine's
/// per-chunk `_add_data` contract.
pub trait DataChunkSource: Debug {
    /// Returns the next chunk of [`Data`], or `None` when the source is exhausted.
    fn next_chunk(&mut self) -> Option<Vec<Data>>;
}

/// Multi-stream, time-ordered data iterator used by the backtest engine.
#[derive(Debug, Default)]
pub struct BacktestDataIterator {
    streams: AHashMap<i32, Vec<Data>>, // key: priority, value: Vec<Data>
    names: AHashMap<i32, String>,      // priority -> name
    priorities: AHashMap<String, i32>, // name -> priority
    indices: AHashMap<i32, usize>,     // cursor per stream
    sources: AHashMap<i32, Box<dyn DataChunkSource>>, // priority -> lazy chunk source (if any)
    heap: BinaryHeap<HeapEntry>,
    single_priority: Option<i32>,
    next_priority_counter: i32, // monotonically increasing counter used to assign priorities
}

impl BacktestDataIterator {
    /// Creates a new empty [`BacktestDataIterator`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            streams: AHashMap::new(),
            names: AHashMap::new(),
            priorities: AHashMap::new(),
            indices: AHashMap::new(),
            sources: AHashMap::new(),
            heap: BinaryHeap::new(),
            single_priority: None,
            next_priority_counter: 0,
        }
    }

    /// Adds (or replaces) a named data stream.
    ///
    /// When `append_data` is true the stream gets lower priority on timestamp
    /// ties; when false (prepend) it wins ties.
    pub fn add_data(&mut self, name: &str, mut data: Vec<Data>, append_data: bool) {
        if data.is_empty() {
            return;
        }

        data.sort_by_key(replay_key);

        self.add_stream(name, data, append_data);
    }

    fn add_stream(&mut self, name: &str, data: Vec<Data>, append_data: bool) {
        self.register_stream(name, data, None, append_data);
    }

    /// Registers a named stream's first chunk and (optionally) the lazy source
    /// that will supply subsequent chunks on demand.
    ///
    /// Shared by [`Self::add_stream`] (materialized streams, `source: None`)
    /// and [`Self::init_data`] (lazy streams).
    fn register_stream(
        &mut self,
        name: &str,
        data: Vec<Data>,
        source: Option<Box<dyn DataChunkSource>>,
        append_data: bool,
    ) {
        let priority = if let Some(p) = self.priorities.get(name) {
            // Replace existing stream – remove previous traces then re-insert below.
            *p
        } else {
            self.next_priority_counter += 1;
            let sign = if append_data { 1 } else { -1 };
            sign * self.next_priority_counter
        };

        // Remove old state if any
        self.remove_data(name, true);

        self.streams.insert(priority, data);
        self.names.insert(priority, name.to_string());
        self.priorities.insert(name.to_string(), priority);
        self.indices.insert(priority, 0);

        if let Some(source) = source {
            self.sources.insert(priority, source);
        }

        self.rebuild_heap();
    }

    /// Registers a named stream backed by a lazily-pulled [`DataChunkSource`].
    ///
    /// Pulls the first chunk eagerly to seed the heap with a valid replay key
    /// (mirroring the legacy Cython engine's `init_data`/`next(data_generator)`),
    /// then stores the source so [`Self::next_item`] can pull subsequent chunks
    /// on demand as the current one is exhausted. If the source yields no data
    /// at all, this is a no-op — matching [`Self::add_data`]'s empty-input behavior.
    pub fn init_data(&mut self, name: &str, mut source: Box<dyn DataChunkSource>, append_data: bool) {
        let Some(mut chunk) = source.next_chunk() else {
            return;
        };

        if chunk.is_empty() {
            return;
        }

        chunk.sort_by_key(replay_key);

        self.register_stream(name, chunk, Some(source), append_data);
    }

    /// Pulls the next chunk for a lazy stream and installs it, replacing the
    /// now-exhausted current chunk. Returns `true` if a non-empty chunk was
    /// installed, or `false` if the source is exhausted (in which case the
    /// stream is fully removed, mirroring the Cython `_update_data`/`StopIteration`
    /// -> `remove_data(complete_remove=True)` path).
    fn refill_stream(&mut self, priority: i32) -> bool {
        let Some(source) = self.sources.get_mut(&priority) else {
            return false;
        };

        match source.next_chunk() {
            Some(mut chunk) if !chunk.is_empty() => {
                chunk.sort_by_key(replay_key);
                self.streams.insert(priority, chunk);
                self.indices.insert(priority, 0);
                true
            }
            _ => {
                if let Some(name) = self.names.get(&priority).cloned() {
                    self.remove_data(&name, true);
                }
                false
            }
        }
    }

    /// Removes a named data stream.
    ///
    /// When `complete_remove` is true, any associated lazy [`DataChunkSource`]
    /// is also dropped — used when a stream is fully exhausted or replaced,
    /// as opposed to a transient removal that may be re-added later.
    pub fn remove_data(&mut self, name: &str, complete_remove: bool) {
        if let Some(priority) = self.priorities.remove(name) {
            self.streams.remove(&priority);
            self.indices.remove(&priority);
            self.names.remove(&priority);

            // Rebuild heap sans removed priority
            self.heap.retain(|e| e.priority != priority);

            if self.heap.is_empty() {
                self.single_priority = None;
            }

            if complete_remove {
                self.sources.remove(&priority);
            }
        }
    }

    /// Sets the cursor of a named stream to `index` (0-based).
    pub fn set_index(&mut self, name: &str, index: usize) {
        if let Some(priority) = self.priorities.get(name) {
            self.indices.insert(*priority, index);
            self.rebuild_heap();
        }
    }

    /// Resets all stream cursors to the beginning.
    pub fn reset_all_cursors(&mut self) {
        for idx in self.indices.values_mut() {
            *idx = 0;
        }
        self.rebuild_heap();
    }

    /// Returns the next backtest data element across all streams in replay order.
    ///
    /// When a stream's currently-loaded chunk is exhausted and it was registered
    /// via [`Self::init_data`], the next chunk is pulled from its [`DataChunkSource`]
    /// on demand before the stream is considered done — mirroring the legacy
    /// Cython engine's generator refill logic (`_update_data`).
    pub(crate) fn next_item(&mut self) -> Option<Data> {
        // Fast path for single stream
        if let Some(p) = self.single_priority {
            let len = self.streams.get(&p)?.len();
            let idx = *self.indices.get(&p)?;

            if idx >= len {
                return None;
            }

            let element = self.streams.get(&p)?[idx].clone();
            let next_idx = idx + 1;
            self.indices.insert(p, next_idx);

            // Eagerly refill at the chunk boundary (mirrors Cython's `_update_data`
            // call placement) so `is_done` stays consistent with actual exhaustion.
            if next_idx >= len {
                self.refill_stream(p);
            }

            return Some(element);
        }

        // Multi-stream path using heap
        let entry = self.heap.pop()?;
        let priority = entry.priority;
        let index = entry.index;
        let element = self.streams.get(&priority)?[index].clone();

        // Advance cursor and push next entry, refilling the stream on demand if exhausted
        let next_index = index + 1;
        self.indices.insert(priority, next_index);
        let stream_len = self.streams.get(&priority)?.len();
        if next_index < stream_len {
            let key = replay_key(&self.streams.get(&priority)?[next_index]);
            self.heap.push(HeapEntry {
                key,
                priority,
                index: next_index,
            });
        } else if self.refill_stream(priority) {
            let first_key = self
                .streams
                .get(&priority)
                .and_then(|v| v.first())
                .map(replay_key);
            if let Some(key) = first_key {
                self.heap.push(HeapEntry {
                    key,
                    priority,
                    index: 0,
                });
            }
        }

        Some(element)
    }

    /// Returns the next market [`Data`] element across all streams in chronological order.
    #[expect(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Option<Data> {
        self.next_item()
    }

    /// Returns whether all streams have been fully consumed.
    #[must_use]
    pub fn is_done(&self) -> bool {
        if let Some(p) = self.single_priority {
            if let Some(idx) = self.indices.get(&p)
                && let Some(vec) = self.streams.get(&p)
            {
                return *idx >= vec.len();
            }
            true
        } else {
            self.heap.is_empty()
        }
    }

    fn rebuild_heap(&mut self) {
        self.heap.clear();

        // Determine if we’re in single-stream mode
        if self.streams.len() == 1 {
            self.single_priority = self.streams.keys().next().copied();
            return;
        }
        self.single_priority = None;

        for (&priority, vec) in &self.streams {
            let idx = *self.indices.get(&priority).unwrap_or(&0);
            if idx < vec.len() {
                self.heap.push(HeapEntry {
                    key: replay_key(&vec[idx]),
                    priority,
                    index: idx,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use nautilus_model::{
        data::QuoteTick,
        identifiers::InstrumentId,
        types::{Price, Quantity},
    };
    #[cfg(feature = "defi")]
    use nautilus_model::{
        defi::{
            DefiData,
            data::block::BlockPosition,
            pool_analysis::snapshot::{PoolAnalytics, PoolSnapshot, PoolState},
        },
        identifiers::{Symbol, Venue},
    };
    use rstest::rstest;

    use super::*;

    fn quote(id: &str, ts: u64) -> Data {
        let inst = InstrumentId::from(id);
        Data::Quote(QuoteTick::new(
            inst,
            Price::from("1.0"),
            Price::from("1.0"),
            Quantity::from(100),
            Quantity::from(100),
            ts.into(),
            ts.into(),
        ))
    }

    fn collect_ts(it: &mut BacktestDataIterator) -> Vec<u64> {
        let mut ts = Vec::new();
        while let Some(d) = it.next() {
            ts.push(d.ts_init().as_u64());
        }
        ts
    }

    #[cfg(feature = "defi")]
    fn defi_snapshot(ts: u64, block: u64, transaction_index: u32, log_index: u32) -> Data {
        let instrument_id = InstrumentId::new(Symbol::from("ETH/USDC"), Venue::from("UNISWAPV3"));
        let snapshot = PoolSnapshot::new(
            instrument_id,
            PoolState::default(),
            Vec::new(),
            Vec::new(),
            PoolAnalytics::default(),
            BlockPosition::new(block, format!("0x{block:x}"), transaction_index, log_index),
            UnixNanos::from(ts),
            UnixNanos::from(ts),
        );

        Data::Defi(Box::new(DefiData::PoolSnapshot(snapshot)))
    }

    #[rstest]
    fn test_single_stream_yields_in_order() {
        let mut it = BacktestDataIterator::new();
        it.add_data(
            "s",
            vec![quote("A.B", 100), quote("A.B", 200), quote("A.B", 300)],
            true,
        );

        assert_eq!(collect_ts(&mut it), vec![100, 200, 300]);
        assert!(it.is_done());
    }

    #[rstest]
    fn test_single_stream_exhaustion_returns_none() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s", vec![quote("A.B", 1), quote("A.B", 3)], true);
        assert_eq!(it.next().unwrap().ts_init(), UnixNanos::from(1));
        assert_eq!(it.next().unwrap().ts_init(), UnixNanos::from(3));
        assert!(it.next().is_none());
    }

    #[rstest]
    fn test_single_stream_sorts_unsorted_input() {
        let mut it = BacktestDataIterator::new();
        it.add_data(
            "s",
            vec![quote("A.B", 300), quote("A.B", 100), quote("A.B", 200)],
            true,
        );

        assert_eq!(collect_ts(&mut it), vec![100, 200, 300]);
    }

    #[rstest]
    fn test_two_stream_merge_chronological() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s1", vec![quote("A.B", 1), quote("A.B", 4)], true);
        it.add_data("s2", vec![quote("C.D", 2), quote("C.D", 3)], false);

        assert_eq!(collect_ts(&mut it), vec![1, 2, 3, 4]);
    }

    #[rstest]
    fn test_three_stream_merge_sorted() {
        let mut it = BacktestDataIterator::new();
        let data_len = 5;
        let d0: Vec<Data> = (0..data_len).map(|k| quote("A.B", 3 * k)).collect();
        let d1: Vec<Data> = (0..data_len).map(|k| quote("C.D", 3 * k + 1)).collect();
        let d2: Vec<Data> = (0..data_len).map(|k| quote("E.F", 3 * k + 2)).collect();
        it.add_data("d0", d0, true);
        it.add_data("d1", d1, true);
        it.add_data("d2", d2, true);

        let ts = collect_ts(&mut it);
        assert_eq!(ts.len(), 15);
        for i in 0..ts.len() - 1 {
            assert!(ts[i] <= ts[i + 1], "Not sorted at index {i}");
        }
    }

    #[rstest]
    fn test_multiple_streams_merge_order() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s1", vec![quote("A.B", 100), quote("A.B", 300)], true);
        it.add_data("s2", vec![quote("C.D", 200), quote("C.D", 400)], true);

        assert_eq!(collect_ts(&mut it), vec![100, 200, 300, 400]);
    }

    #[rstest]
    fn test_append_data_priority_default_fifo() {
        let mut it = BacktestDataIterator::new();
        it.add_data("a", vec![quote("A.B", 100)], true);
        it.add_data("b", vec![quote("C.D", 100)], true);

        // Both at same timestamp, FIFO order (a before b)
        let ts = collect_ts(&mut it);
        assert_eq!(ts, vec![100, 100]);
    }

    #[rstest]
    fn test_prepend_priority_wins_ties() {
        let mut it = BacktestDataIterator::new();
        // "a" is appended (lower priority), "b" is prepended (higher priority)
        it.add_data("a", vec![quote("A.B", 100)], true);
        it.add_data("b", vec![quote("C.D", 100)], false);

        // "b" (prepend) should come first despite being added second
        let first = it.next().unwrap();
        let second = it.next().unwrap();
        // Prepend stream (negative priority) wins ties over append (positive)
        assert_eq!(first.instrument_id(), InstrumentId::from("C.D"));
        assert_eq!(second.instrument_id(), InstrumentId::from("A.B"));
    }

    #[rstest]
    fn test_is_done_empty_iterator() {
        let it = BacktestDataIterator::new();
        assert!(it.is_done());
    }

    #[rstest]
    fn test_is_done_after_consumption() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s", vec![quote("A.B", 1)], true);

        assert!(!it.is_done());
        it.next();
        assert!(it.is_done());
    }

    #[rstest]
    fn test_is_done_multi_stream() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s1", vec![quote("A.B", 1)], true);
        it.add_data("s2", vec![quote("C.D", 2)], true);

        assert!(!it.is_done());
        it.next();
        assert!(!it.is_done());
        it.next();
        assert!(it.is_done());
    }

    #[rstest]
    fn test_partial_consumption_then_complete() {
        let mut it = BacktestDataIterator::new();
        it.add_data(
            "s",
            vec![
                quote("A.B", 0),
                quote("A.B", 1),
                quote("A.B", 2),
                quote("A.B", 3),
            ],
            true,
        );

        assert_eq!(it.next().unwrap().ts_init().as_u64(), 0);
        assert_eq!(it.next().unwrap().ts_init().as_u64(), 1);

        let remaining = collect_ts(&mut it);
        assert_eq!(remaining, vec![2, 3]);
        assert!(it.is_done());
    }

    #[rstest]
    fn test_remove_stream_reduces_output() {
        let mut it = BacktestDataIterator::new();
        it.add_data("a", vec![quote("A.B", 1)], true);
        it.add_data("b", vec![quote("C.D", 2)], true);

        it.remove_data("a", false);

        assert_eq!(collect_ts(&mut it), vec![2]);
    }

    #[rstest]
    fn test_remove_all_streams_yields_empty() {
        let mut it = BacktestDataIterator::new();
        it.add_data("x", vec![quote("A.B", 1)], true);
        it.add_data("y", vec![quote("C.D", 2)], true);

        it.remove_data("x", false);
        it.remove_data("y", false);

        assert!(it.next().is_none());
        assert!(it.is_done());
    }

    #[rstest]
    fn test_remove_nonexistent_stream_is_noop() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s", vec![quote("A.B", 1)], true);

        it.remove_data("nonexistent", false);

        assert_eq!(collect_ts(&mut it), vec![1]);
    }

    #[rstest]
    fn test_remove_after_full_consumption() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s", vec![quote("A.B", 1), quote("A.B", 2)], true);

        collect_ts(&mut it);

        it.remove_data("s", true);
        assert!(it.is_done());
    }

    #[rstest]
    fn test_set_index_rewinds_stream() {
        let mut it = BacktestDataIterator::new();
        it.add_data(
            "s",
            vec![quote("A.B", 10), quote("A.B", 20), quote("A.B", 30)],
            true,
        );

        assert_eq!(it.next().unwrap().ts_init().as_u64(), 10);

        it.set_index("s", 0);

        assert_eq!(collect_ts(&mut it), vec![10, 20, 30]);
    }

    #[rstest]
    fn test_set_index_skips_forward() {
        let mut it = BacktestDataIterator::new();
        it.add_data(
            "s",
            vec![quote("A.B", 10), quote("A.B", 20), quote("A.B", 30)],
            true,
        );

        it.set_index("s", 2);

        assert_eq!(collect_ts(&mut it), vec![30]);
    }

    #[rstest]
    fn test_set_index_nonexistent_stream_is_noop() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s", vec![quote("A.B", 1)], true);

        it.set_index("nonexistent", 0);

        assert_eq!(collect_ts(&mut it), vec![1]);
    }

    #[rstest]
    fn test_reset_all_cursors_single_stream() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s", vec![quote("A.B", 1), quote("A.B", 2)], true);

        collect_ts(&mut it);
        assert!(it.is_done());

        it.reset_all_cursors();
        assert!(!it.is_done());
        assert_eq!(collect_ts(&mut it), vec![1, 2]);
    }

    #[rstest]
    fn test_reset_all_cursors_multi_stream() {
        let mut it = BacktestDataIterator::new();
        it.add_data("s1", vec![quote("A.B", 1), quote("A.B", 3)], true);
        it.add_data("s2", vec![quote("C.D", 2), quote("C.D", 4)], true);

        collect_ts(&mut it);
        assert!(it.is_done());

        it.reset_all_cursors();
        assert_eq!(collect_ts(&mut it), vec![1, 2, 3, 4]);
    }

    #[rstest]
    fn test_readding_data_replaces_stream() {
        let mut it = BacktestDataIterator::new();
        it.add_data("X", vec![quote("A.B", 1), quote("A.B", 2)], true);
        it.add_data("X", vec![quote("A.B", 10)], true);

        assert_eq!(collect_ts(&mut it), vec![10]);
    }

    #[rstest]
    fn test_add_empty_data_is_noop() {
        let mut it = BacktestDataIterator::new();
        it.add_data("empty", vec![], true);

        assert!(it.is_done());
        assert!(it.next().is_none());
    }

    #[rstest]
    fn test_empty_iterator_returns_none() {
        let mut it = BacktestDataIterator::new();
        assert!(it.next().is_none());
        assert!(it.is_done());
    }

    #[rstest]
    fn test_multiple_add_data_calls_with_different_names() {
        let mut it = BacktestDataIterator::new();
        it.add_data("batch_0", vec![quote("A.B", 1), quote("A.B", 3)], true);
        it.add_data("batch_1", vec![quote("A.B", 2), quote("A.B", 4)], true);

        assert_eq!(collect_ts(&mut it), vec![1, 2, 3, 4]);
    }

    #[rstest]
    fn test_prepend_stream_always_wins_ties_across_batches() {
        // Verifies that a prepend stream (negative priority) wins ties
        // even when added after multiple append streams
        let mut it = BacktestDataIterator::new();
        it.add_data("append_a", vec![quote("A.B", 100)], true);
        it.add_data("append_b", vec![quote("C.D", 100)], true);
        it.add_data("prepend", vec![quote("E.F", 100)], false);

        let first = it.next().unwrap();
        assert_eq!(
            first.instrument_id(),
            InstrumentId::from("E.F"),
            "Prepend stream should always come first in ties"
        );
    }

    #[rstest]
    fn test_equal_timestamps_across_many_streams_preserves_priority_order() {
        // All items at the same timestamp — ordering is strictly by priority
        let mut it = BacktestDataIterator::new();
        it.add_data("s1", vec![quote("A.B", 50)], true);
        it.add_data("s2", vec![quote("C.D", 50)], true);
        it.add_data("s3", vec![quote("E.F", 50)], true);
        it.add_data("s4", vec![quote("G.H", 50)], true);

        let mut ids = Vec::new();
        while let Some(d) = it.next() {
            ids.push(d.instrument_id());
        }

        assert_eq!(ids.len(), 4);

        // All should be yielded (no duplicates dropped, no items lost)
        assert!(ids.contains(&InstrumentId::from("A.B")));
        assert!(ids.contains(&InstrumentId::from("C.D")));
        assert!(ids.contains(&InstrumentId::from("E.F")));
        assert!(ids.contains(&InstrumentId::from("G.H")));
    }

    #[cfg(feature = "defi")]
    #[rstest]
    fn test_defi_data_orders_equal_timestamps_by_block_position() {
        let mut it = BacktestDataIterator::new();
        it.add_data(
            "defi",
            vec![
                defi_snapshot(100, 12, 4, 1),
                defi_snapshot(100, 11, 9, 9),
                defi_snapshot(100, 12, 2, 7),
            ],
            true,
        );

        let mut positions = Vec::new();
        while let Some(Data::Defi(data)) = it.next_item() {
            positions.push(data.block_position());
        }

        assert_eq!(positions, vec![(11, 9, 9), (12, 2, 7), (12, 4, 1)]);
    }

    /// A mock [`DataChunkSource`] that lazily yields pre-built fixed-size chunks,
    /// mirroring the behavior of a generator pulling from an external store.
    ///
    /// Exposes a shared pull counter so tests can assert chunks are pulled
    /// incrementally on demand rather than all materialized upfront.
    #[derive(Debug)]
    struct MockChunkSource {
        chunks: Vec<Vec<Data>>,
        cursor: usize,
        pull_count: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl MockChunkSource {
        fn new(chunks: Vec<Vec<Data>>) -> Self {
            Self {
                chunks,
                cursor: 0,
                pull_count: std::rc::Rc::new(std::cell::Cell::new(0)),
            }
        }

        fn with_counter(chunks: Vec<Vec<Data>>, pull_count: std::rc::Rc<std::cell::Cell<usize>>) -> Self {
            Self {
                chunks,
                cursor: 0,
                pull_count,
            }
        }
    }

    impl DataChunkSource for MockChunkSource {
        fn next_chunk(&mut self) -> Option<Vec<Data>> {
            self.pull_count.set(self.pull_count.get() + 1);
            if self.cursor >= self.chunks.len() {
                return None;
            }
            let chunk = self.chunks[self.cursor].clone();
            self.cursor += 1;
            Some(chunk)
        }
    }

    fn lazy_quotes(id: &str, timestamps: &[u64], chunk_size: usize) -> Vec<Vec<Data>> {
        timestamps
            .iter()
            .map(|&ts| quote(id, ts))
            .collect::<Vec<_>>()
            .chunks(chunk_size)
            .map(<[Data]>::to_vec)
            .collect()
    }

    #[rstest]
    fn test_lazy_stream_matches_eager_equivalent_order() {
        let timestamps: Vec<u64> = (0..20).map(|k| k * 10).collect();

        let mut lazy_it = BacktestDataIterator::new();
        lazy_it.init_data(
            "lazy",
            Box::new(MockChunkSource::new(lazy_quotes("A.B", &timestamps, 3))),
            true,
        );

        let mut eager_it = BacktestDataIterator::new();
        eager_it.add_data(
            "eager",
            timestamps.iter().map(|&ts| quote("A.B", ts)).collect(),
            true,
        );

        assert_eq!(collect_ts(&mut lazy_it), collect_ts(&mut eager_it));
    }

    #[rstest]
    fn test_lazy_stream_is_exhausted_after_all_chunks_consumed() {
        let timestamps: Vec<u64> = (0..7).map(|k| k * 10).collect();
        let mut it = BacktestDataIterator::new();
        it.init_data(
            "lazy",
            Box::new(MockChunkSource::new(lazy_quotes("A.B", &timestamps, 3))),
            true,
        );

        assert_eq!(
            collect_ts(&mut it),
            timestamps.clone(),
            "lazy stream should yield every element across all chunks in order"
        );
        assert!(it.is_done());
        assert!(it.next().is_none());
    }

    #[rstest]
    fn test_lazy_and_materialized_streams_merge_chronologically() {
        let lazy_timestamps: Vec<u64> = vec![1, 4, 7, 10, 13];
        let mut it = BacktestDataIterator::new();
        it.init_data(
            "lazy",
            Box::new(MockChunkSource::new(lazy_quotes(
                "A.B",
                &lazy_timestamps,
                2,
            ))),
            true,
        );
        it.add_data(
            "materialized",
            vec![
                quote("C.D", 2),
                quote("C.D", 5),
                quote("C.D", 8),
                quote("C.D", 11),
            ],
            true,
        );

        assert_eq!(collect_ts(&mut it), vec![1, 2, 4, 5, 7, 8, 10, 11, 13]);
        assert!(it.is_done());
    }

    #[rstest]
    fn test_lazy_stream_sorts_unsorted_chunks() {
        let mut it = BacktestDataIterator::new();
        it.init_data(
            "lazy",
            Box::new(MockChunkSource::new(vec![
                vec![quote("A.B", 300), quote("A.B", 100), quote("A.B", 200)],
                vec![quote("A.B", 600), quote("A.B", 400), quote("A.B", 500)],
            ])),
            true,
        );

        assert_eq!(collect_ts(&mut it), vec![100, 200, 300, 400, 500, 600]);
    }

    #[rstest]
    fn test_init_data_with_immediately_exhausted_source_is_noop() {
        let mut it = BacktestDataIterator::new();
        it.init_data("lazy", Box::new(MockChunkSource::new(vec![])), true);

        assert!(it.is_done());
        assert!(it.next().is_none());
    }

    #[rstest]
    fn test_init_data_with_empty_first_chunk_is_noop() {
        let mut it = BacktestDataIterator::new();
        it.init_data(
            "lazy",
            Box::new(MockChunkSource::new(vec![vec![], vec![quote("A.B", 1)]])),
            true,
        );

        // Mirrors `add_data`'s empty-input behavior: an empty first chunk means
        // the stream is never registered, even if subsequent chunks are non-empty.
        assert!(it.is_done());
        assert!(it.next().is_none());
    }

    #[rstest]
    fn test_lazy_stream_pulls_chunks_incrementally_not_all_upfront() {
        let timestamps: Vec<u64> = (0..10).map(|k| k * 10).collect();
        let pull_count = std::rc::Rc::new(std::cell::Cell::new(0));
        let source =
            MockChunkSource::with_counter(lazy_quotes("A.B", &timestamps, 2), pull_count.clone());

        let mut it = BacktestDataIterator::new();
        it.init_data("lazy", Box::new(source), true);

        // `init_data` pulls only the first chunk to seed the heap.
        assert_eq!(pull_count.get(), 1);

        // Consuming the first chunk's two elements triggers exactly one more pull
        // (for the second chunk) — not a pull of the entire remaining series.
        assert_eq!(it.next().unwrap().ts_init().as_u64(), 0);
        assert_eq!(it.next().unwrap().ts_init().as_u64(), 10);
        assert_eq!(pull_count.get(), 2);

        assert_eq!(it.next().unwrap().ts_init().as_u64(), 20);
        assert_eq!(it.next().unwrap().ts_init().as_u64(), 30);
        assert_eq!(pull_count.get(), 3);

        let remaining = collect_ts(&mut it);
        assert_eq!(remaining, vec![40, 50, 60, 70, 80, 90]);
        assert!(it.is_done());

        // Final pull discovers exhaustion (5 chunks of 2 -> 5 data pulls + 1 exhaustion probe).
        assert_eq!(pull_count.get(), 6);
    }

    #[rstest]
    fn test_remove_lazy_stream_drops_source() {
        let mut it = BacktestDataIterator::new();
        it.init_data(
            "lazy",
            Box::new(MockChunkSource::new(lazy_quotes("A.B", &[1, 2, 3], 1))),
            true,
        );
        it.add_data("other", vec![quote("C.D", 100)], true);

        let priority = *it.priorities.get("lazy").unwrap();
        assert!(it.sources.contains_key(&priority));

        it.remove_data("lazy", true);

        assert!(!it.sources.contains_key(&priority));
        assert_eq!(collect_ts(&mut it), vec![100]);
    }
}
