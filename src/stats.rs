//! 統計値取得用のプリミティブ型

use std::sync::atomic::{AtomicU64, Ordering};

/// 単調増加カウンター
///
/// 通算値を保持する。`AtomicU64` の薄いラッパーであり、
/// 共有が必要な場合はカウンターを含む構造体 (`DecoderStats` / `EncoderStats` 等) を
/// `Arc` で包んで行う。ワーカスレッドが `inc()` でインクリメントし、
/// 利用側が `get()` で読み出す。
///
/// カウンターは純粋な累積値であり、スレッド間の happens-before 関係を
/// 要求しないため、すべての操作を relaxed order で行う。
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// 0 で初期化したカウンターを生成する
    pub fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 現在の値を読み出す
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// カウンターを 1 増やす
    pub fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// カウンターに `n` を加算する
    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }
}

impl Clone for Counter {
    /// 現在値のスナップショットを取得する
    ///
    /// 共有には `Arc` を使うため、`clone()` は独立したスナップショットを返す
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.get()))
    }
}

/// 現在値を表すゲージ
///
/// 時点値を保持する。`AtomicU64` の薄いラッパーであり、単調増加する
/// 通算値 ([`Counter`]) と異なり、値を設定して現在値を表す。
/// 生成時に確定する値や、増減する現在値の保持に使う。
///
/// ゲージは純粋な時点値であり、スレッド間の happens-before 関係を
/// 要求しないため、すべての操作を relaxed order で行う。
#[derive(Debug, Default)]
pub struct Gauge(AtomicU64);

impl Gauge {
    /// 0 で初期化したゲージを生成する
    pub fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 現在の値を読み出す
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// 値を設定する
    pub fn set(&self, value: u64) {
        self.0.store(value, Ordering::Relaxed);
    }
}

impl Clone for Gauge {
    /// 現在値のスナップショットを取得する
    ///
    /// 共有には `Arc` を使うため、`clone()` は独立したスナップショットを返す
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.get()))
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

    /// clone したカウンターは現在値のスナップショットであり、以後の変更は互いに影響しない
    #[test]
    fn counter_clone_is_independent_snapshot() {
        let counter = Counter::new();
        counter.add(5);

        let snapshot = counter.clone();

        counter.inc();
        assert_eq!(counter.get(), 6);
        assert_eq!(snapshot.get(), 5);
    }

    /// 生成直後のゲージは 0 である
    #[test]
    fn gauge_new_is_zero() {
        let gauge = Gauge::new();
        assert_eq!(gauge.get(), 0);
    }

    /// set() で値を設定できる
    #[test]
    fn gauge_set_updates_value() {
        let gauge = Gauge::new();
        gauge.set(42);
        assert_eq!(gauge.get(), 42);
    }

    /// clone したゲージは現在値のスナップショットであり、以後の変更は互いに影響しない
    #[test]
    fn gauge_clone_is_independent_snapshot() {
        let gauge = Gauge::new();
        gauge.set(10);

        let snapshot = gauge.clone();

        gauge.set(20);
        assert_eq!(gauge.get(), 20);
        assert_eq!(snapshot.get(), 10);
    }
}
