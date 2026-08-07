# 0027-add-decoder-encoder-stats-api

- Created: 2026-08-07
- Branch: feature/add-decoder-encoder-stats-api

## 目的

`Decoder` / `Encoder` の内部で発生した各種イベントの通算回数 (カウンター) を、利用側から統一的な API で取得できるようにする。

利用側 (hisui 等) はメトリクス収集・アラート・テストでの挙動検証にこの情報を使う。個別 API を都度生やすのではなく、`Stats` 構造体を返す統一 API として設計する。

## 現状

`Decoder` / `Encoder` の内部状態 (デコーダー作成回数、reconfigure 回数、失敗回数、encoder buffer full 発生回数等) は外部から観測できない。

- 0024 の作業ブランチで `Decoder` に `#[cfg(test)]` 限定で `create_decoder_count` / `reconfigure_decoder_count` を追加したが、pub 化されていないため本番用途 (メトリクス収集) では使えない
- 利用側で「reconfigure 失敗が何回起きたか」を集計してアラートしたいが手段がない
- 利用側でテストを書くときに「reconfigure が呼ばれたか」「destroy+create が使われたか」を検証したいが、直接観測する手段がない (frame 数から間接的に推測するしかない)
- `Encoder` 側は `encoder buffer is full` エラーの発生回数 (`Error::new_custom("encode", "encoder buffer is full")` 該当箇所) を外側から集計できない

なお 0026 (`add-decoder-reconfigure-failure-observability`) で `Decoder::reconfigure_failure_count()` の個別 API を追加する予定だったが、本 issue で統一 API に吸収することとし 0026 は close する。

## 設計方針

### API 形

`Decoder::stats(&self) -> DecoderStats` / `Encoder::stats(&self) -> EncoderStats` の統一メソッドを追加し、`DecoderStats` / `EncoderStats` 構造体を返す。

```rust
#[derive(Debug, Clone)]
pub struct DecoderStats {
    /// cuvidCreateDecoder の通算呼び出し回数
    pub create_decoder_count: u64,

    /// cuvidReconfigureDecoder の通算呼び出し回数 (成功のみ)
    pub reconfigure_decoder_count: u64,

    /// cuvidReconfigureDecoder の通算失敗回数
    pub reconfigure_failure_count: u64,
}
```

`Encoder` 側も同様の構造体を用意する。少なくとも `encoder buffer is full` の発生回数 (`encoder_buffer_full_count` 等) を含める。他の項目は本 issue の設計段階でソースコードを再確認して洗い出す。

### 実装方式

`DecoderState` / `EncoderState` に `AtomicU64` フィールドを持たせ、該当箇所でインクリメントする。`stats()` は各 atomic を relaxed order で読み出す。

- **軽量である必要がある**: メトリクス収集で頻繁に呼ばれても影響を出さないため atomic load
- **ロックフリー**: 利用側スレッドとワーカスレッド間の同期を最小化
- ワーカ経由 (`Job::QueryStats` 的な RPC) は overhead が大きいので採用しない

0024 の作業ブランチで追加した `#[cfg(test)]` 限定の `create_decoder_count` / `reconfigure_decoder_count` は本 API に統合し、`#[cfg(test)]` を外して pub 提供する。`Job::QueryCallCounts` の RPC 経路も不要になる (削除)。

### 命名

- `Decoder::stats()` / `Encoder::stats()` の統一メソッド名
- 個別カウンターは `xxx_count` (通算回数) で統一
- 将来 gauge 系 (「現在の in-flight 数」等) が入ったら別構造体に分けるか、`current_xxx` 等の prefix で区別するかは本 issue のスコープ外 (追加が必要になった時点で判断)

## 完了条件

- `Decoder::stats() -> DecoderStats` が pub で追加され、以下 3 カウンターが取得できる:
  - `create_decoder_count`
  - `reconfigure_decoder_count`
  - `reconfigure_failure_count`
- `Encoder::stats() -> EncoderStats` が pub で追加され、少なくとも `encoder_buffer_full_count` が取得できる
- 0024 の作業ブランチで追加した `#[cfg(test)]` 限定の `create_decoder_count` / `reconfigure_decoder_count` と `Job::QueryCallCounts` が削除され、本 API に統合されている
- カウンター取得 API の単体テスト or 結合テストがある
- `CHANGES.md` に `[ADD]` エントリが追加されている
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` に統計値取得の例が追記されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — `DecoderStats` 定義、`DecoderState` に `AtomicU64` フィールド追加、`Decoder::stats()` 追加、既存 `#[cfg(test)]` カウンターと `Job::QueryCallCounts` の削除
- `src/encode.rs` — `EncoderStats` 定義、`EncoderState` に `AtomicU64` フィールド追加、`Encoder::stats()` 追加、`encoder buffer is full` 発生箇所でインクリメント
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` — 統計値取得節の追加
- `CHANGES.md` — 追記例:
  - `- [ADD] Decoder::stats() / Encoder::stats() で内部カウンターを取得できるようにする`
  - `  - @担当者`

## 関連 issue

- 0024: `#[cfg(test)]` カウンターの追加元。本 issue で pub 化して統合
- 0026 (close 予定): `Decoder::reconfigure_failure_count()` の個別 API 案。本 issue に吸収して close
