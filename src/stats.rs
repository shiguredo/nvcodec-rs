//! 統計値取得用のプリミティブ型

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// 共有可能な単調増加カウンター
///
/// 統計値はすべてこの型で表現する。内部で `Arc<AtomicU64>` を持ち、
/// `clone()` で同じカウンターを共有できる。ワーカスレッドが `inc()` で
/// インクリメントし、利用側が `get()` で読み出す。
///
/// カウンターは純粋な累積値であり、スレッド間の happens-before 関係を
/// 要求しないため、すべての操作を relaxed order で行う。
#[derive(Debug, Clone, Default)]
pub struct Counter(Arc<AtomicU64>);

impl Counter {
    /// 0 で初期化したカウンターを生成する
    pub fn new() -> Self {
        Self(Arc::new(AtomicU64::new(0)))
    }

    /// 現在の値を読み出す
    ///
    /// relaxed order の atomic load で、単純な `u64` のスナップショットを返す
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// カウンターを 1 増やす
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// カウンターに `n` を加算する
    ///
    /// 生成時に確定する値 (例: `max_in_flight_frames`) の初期化に使う
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 生成直後のカウンターは 0 である
    #[test]
    fn counter_new_is_zero() {
        let counter = Counter::new();
        assert_eq!(counter.get(), 0);
    }

    /// inc() を呼ぶたびに 1 ずつ増える
    #[test]
    fn counter_inc_increments_by_one() {
        let counter = Counter::new();
        counter.inc();
        counter.inc();
        counter.inc();
        assert_eq!(counter.get(), 3);
    }

    /// add() で指定した値が加算される
    #[test]
    fn counter_add_increments_by_n() {
        let counter = Counter::new();
        counter.add(5);
        counter.add(7);
        assert_eq!(counter.get(), 12);
    }

    /// clone したカウンターは同じ値を共有する
    #[test]
    fn counter_clone_shares_value() {
        let counter = Counter::new();
        let counter_clone = counter.clone();

        counter_clone.inc();
        counter.inc();

        assert_eq!(counter_clone.get(), 2);
        assert_eq!(counter.get(), 2);
    }
}
