# 0027-add-decoder-encoder-stats-api

- Created: 2026-08-07
- Branch: feature/add-decoder-encoder-stats-api

## 目的

`Decoder` / `Encoder` の内部状態を利用側から統一的な API で取得できるようにする。

対象は observability 目的で見たい値全般で、以下 2 種類を同じ `Stats` 構造体に含める。

- **counter**: monotonically increasing な通算値 (`create_decoder_count` 等)
- **gauge**: 現在値または encoder ライフサイクル中変わらない静的な値 (`max_in_flight_frames` 等)

利用側 (hisui 等) はメトリクス収集・アラート・テストでの挙動検証・in-flight 制御にこの情報を使う。個別 API を都度生やすのではなく、`Stats` 構造体を返す統一 API として設計する。

## 現状

`Decoder` / `Encoder` の内部状態 (デコーダー作成回数、reconfigure 回数、失敗回数、encoder buffer full 発生回数、encoder の in-flight 上限値等) は外部から観測できない。

- 0024 の作業ブランチで `Decoder` に `#[cfg(test)]` 限定で `create_decoder_count` / `reconfigure_decoder_count` を追加したが、pub 化されていないため本番用途 (メトリクス収集) では使えない
- 利用側で「reconfigure 失敗が何回起きたか」を集計してアラートしたいが手段がない
- 利用側でテストを書くときに「reconfigure が呼ばれたか」「destroy+create が使われたか」を検証したいが、直接観測する手段がない (frame 数から間接的に推測するしかない)
- `Encoder` 側は `encoder buffer is full` エラーの発生回数 (`Error::new_custom("encode", "encoder buffer is full")` 該当箇所) を外側から集計できない
- `Encoder` の `n_encoder_buffer` (= `frame_interval_p + 3`) は private フィールドで、利用側が「`encoder buffer is full` を避けるために何フレームで flush すべきか」を知る手段がない (hisui では現状 `frame_interval_p + 2` を利用側で再計算している)

なお 0026 (`add-decoder-reconfigure-failure-observability`) で `Decoder::reconfigure_failure_count()` の個別 API を追加する予定だったが、本 issue で統一 API に吸収することとし 0026 は close する。

## 設計方針

### API 形

`Decoder::stats(&self) -> DecoderStats` / `Encoder::stats(&self) -> EncoderStats` の統一メソッドを追加し、`DecoderStats` / `EncoderStats` 構造体を返す。counter と gauge を同じ構造体に含める。

```rust
#[derive(Debug, Clone)]
pub struct DecoderStats {
    // counter (通算値)
    /// cuvidCreateDecoder の通算呼び出し回数
    pub create_decoder_count: u64,

    /// cuvidReconfigureDecoder の通算呼び出し回数 (成功のみ)
    pub reconfigure_decoder_count: u64,

    /// cuvidReconfigureDecoder の通算失敗回数
    pub reconfigure_failure_count: u64,
}

#[derive(Debug, Clone)]
pub struct EncoderStats {
    // counter
    /// "encoder buffer is full" エラーの通算発生回数
    pub encoder_buffer_full_count: u64,

    // gauge (encoder のライフサイクル中変わらない静的な値)
    /// 利用側が flush せずに encode を連続呼び出しできる最大フレーム数
    /// (n_encoder_buffer - 1 = frame_interval_p + 2)
    pub max_in_flight_frames: u32,
}
```

含める項目は本 issue の設計段階で `src/encode.rs` / `src/decode.rs` を再確認して最終確定する。将来「今この瞬間の in-flight 数」等の動的 gauge を追加する余地は残す。

### counter / gauge の区別

命名規約で区別する (Prometheus / OpenMetrics 相当の運用):

- counter: `xxx_count` (通算数)
- gauge: `current_xxx` (時点値) / `max_xxx` / `xxx_frames` 等 (意味に応じた命名)

型で分けない (`u64` / `u32` は値域で選ぶ、意味論とは無関係)。呼び出し側での区別は rustdoc とフィールド名でカバーする。

### 実装方式

- **counter**: `DecoderState` / `EncoderState` に `AtomicU64` フィールドを持たせ、該当箇所でインクリメント。`stats()` は各 atomic を relaxed order で読み出す
- **gauge (静的)**: encoder / decoder 生成時に確定する値は、`Encoder` / `Decoder` 構造体の `u32` フィールドとして持たせ、cheap read で返す (atomic 不要)
- **gauge (動的)**: 現状スコープ外だが、追加する場合は同じ atomic 手法で扱う想定

いずれも以下の要件を満たす:

- **軽量である必要がある**: メトリクス収集で頻繁に呼ばれても影響を出さないため atomic load or plain field read
- **ロックフリー**: 利用側スレッドとワーカスレッド間の同期を最小化
- ワーカ経由 (`Job::QueryStats` 的な RPC) は overhead が大きいので採用しない

0024 の作業ブランチで追加した `#[cfg(test)]` 限定の `create_decoder_count` / `reconfigure_decoder_count` は本 API に統合し、`#[cfg(test)]` を外して pub 提供する。`Job::QueryCallCounts` の RPC 経路も不要になる (削除)。

### 命名

- `Decoder::stats()` / `Encoder::stats()` の統一メソッド名 (counter / gauge を含めた総称)
- counter は `xxx_count` (通算回数) で統一
- gauge は用途に応じた命名 (`max_xxx` / `current_xxx` / `xxx_frames` 等)

## 完了条件

- `Decoder::stats() -> DecoderStats` が pub で追加され、以下 3 counter が取得できる:
  - `create_decoder_count`
  - `reconfigure_decoder_count`
  - `reconfigure_failure_count`
- `Encoder::stats() -> EncoderStats` が pub で追加され、以下の 2 項目が取得できる:
  - counter: `encoder_buffer_full_count`
  - gauge: `max_in_flight_frames`
- 0024 の作業ブランチで追加した `#[cfg(test)]` 限定の `create_decoder_count` / `reconfigure_decoder_count` と `Job::QueryCallCounts` が削除され、本 API に統合されている
- 単体テスト or 結合テストがある (counter のインクリメント検証 + gauge の値検証)
- `CHANGES.md` に `[ADD]` エントリが追加されている
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` に統計値取得の例と「in-flight 上限に基づく flush 制御」のレシピが追記されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — `DecoderStats` 定義、`DecoderState` に `AtomicU64` フィールド追加、`Decoder::stats()` 追加、既存 `#[cfg(test)]` カウンターと `Job::QueryCallCounts` の削除
- `src/encode.rs` — `EncoderStats` 定義、`EncoderState` に `AtomicU64` フィールド追加、`Encoder` に `max_in_flight_frames: u32` フィールド追加、`Encoder::stats()` 追加、`encoder buffer is full` 発生箇所でインクリメント
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` — 統計値取得節の追加 + in-flight 制御のレシピ
- `CHANGES.md` — 追記例:
  - `- [ADD] Decoder::stats() / Encoder::stats() で内部状態 (counter / gauge) を取得できるようにする`
  - `  - @担当者`

## 関連 issue

- 0024: `#[cfg(test)]` カウンターの追加元。本 issue で pub 化して統合
- 0026 (closed): `Decoder::reconfigure_failure_count()` の個別 API 案。本 issue に吸収して close 済み
