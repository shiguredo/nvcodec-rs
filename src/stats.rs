//! 統計値取得用のプリミティブ型

use std::sync::atomic::{AtomicU64, Ordering};

/// 単調増加する通算カウンター
///
/// `AtomicU64` の薄いラッパー。カウンターは純粋な累積値であり、
/// スレッド間の happens-before 関係を要求しないため、すべての操作を
/// relaxed order で行う。
///
/// ワーカスレッドが `inc()` でインクリメントし、利用側が `get()` で
/// 読み出す。`Arc<Counter>` で共有することで、ワーカスレッドと
/// 利用側の両方から同じカウンターにアクセスできる。
///
/// 将来、現在値を表す gauge が必要になった場合は、このモジュールに
/// `Gauge` 型 (`AtomicU64` / `AtomicU32` の薄いラッパー) を追加する想定。
#[derive(Debug, Default)]
pub(crate) struct Counter(AtomicU64);

impl Counter {
    /// 0 で初期化したカウンターを生成する
    pub(crate) const fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 現在の値を読み出す
    ///
    /// relaxed order の atomic load で、単純な `u64` のスナップショットを返す
    pub(crate) fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// カウンターを 1 増やす
    pub(crate) fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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

    /// Arc で共有したカウンターは両方の参照から同じ値が見える
    #[test]
    fn counter_shared_via_arc() {
        let counter = Arc::new(Counter::new());
        let counter_clone = counter.clone();

        counter_clone.inc();
        counter.inc();

        assert_eq!(counter_clone.get(), 2);
        assert_eq!(counter.get(), 2);
    }
}
