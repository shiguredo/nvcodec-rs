# 0027-add-decoder-encoder-stats-api

- Created: 2026-08-07
- Branch: feature/add-decoder-encoder-stats-api
- Polished: 2026-08-10

## 目的

`Decoder` / `Encoder` の内部状態を利用側から統一的な API で取得できるようにする。

対象は observability 目的で見たい値全般で、以下 2 種類を同じ `Stats` 構造体に含める。

- **counter**: monotonically increasing な通算値 (`create_decoder_count` 等)
- **gauge**: 現在値または encoder ライフサイクル中変わらない静的な値 (`max_in_flight_frames` 等)

利用側 (hisui 等) はメトリクス収集・アラート・テストでの挙動検証・in-flight 制御にこの情報を使う。個別 API を都度生やすのではなく、`Stats` 構造体を返す統一 API として設計する。

## 現状

`Decoder` / `Encoder` の内部状態 (デコーダー作成回数、reconfigure 回数、失敗回数、encoder buffer full 発生回数、encoder の in-flight 上限値等) は外部から観測できない。

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
    /// cuvidCreateDecoder の通算成功回数 (初回の create を含む)
    pub create_decoder_count: u64,

    /// cuvidReconfigureDecoder の通算成功回数
    pub reconfigure_decoder_count: u64,

    /// cuvidReconfigureDecoder 呼び出しの通算失敗回数
    /// (解像度上限超過の事前検証エラーや cuvidCreateDecoder の失敗は含まない)
    pub reconfigure_failure_count: u64,
}

#[derive(Debug, Clone)]
pub struct EncoderStats {
    // counter
    /// "encoder buffer is full" エラーの通算発生回数
    pub encoder_buffer_full_count: u64,

    // gauge (encoder のライフサイクル中変わらない静的な値)
    /// "encoder buffer is full" エラーを発生させずに in-flight にできる最大フレーム数
    /// (n_encoder_buffer - 1 = frame_interval_p + 2)
    pub max_in_flight_frames: u32,
}
```

### 統計項目一覧

#### 初版 (本 issue で実装)

| 構造体 | 種別 | フィールド | 説明 |
|---|---|---|---|
| `DecoderStats` | counter | `create_decoder_count` | cuvidCreateDecoder の通算成功回数 (初回の create を含む) |
| `DecoderStats` | counter | `reconfigure_decoder_count` | cuvidReconfigureDecoder の通算成功回数 |
| `DecoderStats` | counter | `reconfigure_failure_count` | cuvidReconfigureDecoder 呼び出しの通算失敗回数 |
| `EncoderStats` | counter | `encoder_buffer_full_count` | "encoder buffer is full" エラーの通算発生回数 |
| `EncoderStats` | gauge (静的) | `max_in_flight_frames` | "encoder buffer is full" エラーを発生させずに in-flight にできる最大フレーム数 (`n_encoder_buffer - 1` = `frame_interval_p + 2`) |

#### 将来拡張候補 (本 issue では実装しない)

| 構造体 | 種別 | フィールド | 説明 |
|---|---|---|---|
| `DecoderStats` | counter | `decode_picture_count` | cuvidDecodePicture の通算成功回数 (デコードフレーム数) |
| `DecoderStats` | counter | `decode_failure_count` | cuvidDecodePicture の通算失敗回数 |
| `DecoderStats` | counter | `output_frame_count` | 出力フレーム数 (display コールバックの通算回数) |
| `DecoderStats` | gauge (動的) | `width` / `height` | 現在のデコード解像度 |
| `EncoderStats` | counter | `encode_count` | encode_frame の通算成功回数 (送信数) |
| `EncoderStats` | counter | `encode_failure_count` | encode_frame の通算失敗回数 |
| `EncoderStats` | counter | `encoded_frame_count` | エンコード出力 (drain 完了) の通算フレーム数 |

補足:

- `current_in_flight_frames` のような「他の counter の差分で導出できる gauge」は独立して提供しない (Prometheus / OpenMetrics 運用では total の差分で求められる gauge を別途提供しないことが多い)
  - in-flight フレーム数は `encode_count - encoded_frame_count` で導出できる
- エラー系カウンターの一部は 0029 (エラーチャネル分離) と絡むため、追加するかは 0029 の状況を見て判断する

### counter / gauge の区別

命名規約で区別する (Prometheus / OpenMetrics 相当の運用):

- counter: `xxx_count` (通算数)
- gauge: `current_xxx` (時点値) / `max_xxx` / `xxx_frames` 等 (意味に応じた命名)

型で分けない (`u64` / `u32` は値域で選ぶ、意味論とは無関係)。呼び出し側での区別は rustdoc とフィールド名でカバーする。

### 実装方式

- **counter**: 新規 `src/stats.rs` に `Counter` 型 (`AtomicU64` の薄いラッパー、`const fn new()` / `get()` / `inc()`、すべて relaxed order) を追加し、`DecoderState` / `EncoderState` と `Decoder` / `Encoder` (pub 構造体) が `Arc<Counter>` で共有する。worker スレッド側が `inc()` でインクリメントし、`stats()` は各 counter の `get()` で読み出して値型の `DecoderStats` / `EncoderStats` を組み立てて返す
- **gauge (静的)**: encoder 生成時に確定する値は、`Encoder` 構造体の `u32` フィールドとして持たせ、cheap read で返す (atomic 不要)
- **gauge (動的)**: 現状スコープ外だが、追加する場合は `Counter` と同様に `src/stats.rs` に `Gauge` 型 (`AtomicU64` / `AtomicU32` の薄いラッパー) を追加して対応する想定
- **呼び出しスレッド**: `Decoder` / `Encoder` はフィールド (`SyncSender` / `Sender` / `Option<JoinHandle>` / `Arc<Counter>` / `u32`) がすべて `Sync` のため、自動導出で既に `Sync` である (unsafe impl の追加は不要)。`stats()` は `&self` で呼べるため、`Arc<Decoder>` / `Arc<Encoder>` をメトリクス収集スレッド等へ共有すれば他スレッドから呼べる

いずれも以下の要件を満たす:

- **軽量である必要がある**: メトリクス収集で頻繁に呼ばれても影響を出さないため atomic load or plain field read
- **ロックフリー**: 利用側スレッドとワーカスレッド間の同期を最小化
- ワーカ経由 (`Job::QueryStats` 的な RPC) は overhead が大きいので採用しない

なお本 issue は develop ベースで実装する (0024 のマージを待たない)。develop には reconfigure 経路が存在しないため、`create_decoder_count` は destroy+create 経路 (`handle_video_sequence_inner`) にインクリメントを置き、`reconfigure_decoder_count` / `reconfigure_failure_count` は API 定義のみ行う。インクリメントは 0024 が develop にマージされた時点で 0024 側で追加する。

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
- counter は `src/stats.rs` の `Counter` 型 (`AtomicU64` の薄いラッパー) で実装され、`stats()` がロックフリーかつ軽量である
- `Decoder` / `Encoder` は既に `Sync` であり、他スレッド (メトリクス収集スレッド等) から `&Decoder` / `&Encoder` 経由で `stats()` を呼べる
- 単体テスト or 結合テストがある (`create_decoder_count` / `encoder_buffer_full_count` のインクリメント検証 + `max_in_flight_frames` の値検証)
- `CHANGES.md` に `[ADD]` エントリが追加されている
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` に統計値取得の例と「in-flight 上限に基づく flush 制御」のレシピが追記されている

なお `reconfigure_decoder_count` / `reconfigure_failure_count` のインクリメントは 0024 の reconfigure 経路に依存するため、本 issue では API 定義のみとし、インクリメントは 0024 の develop マージ時に 0024 側で追加する (0024 マージまでは値が 0 のまま)。

## 解決方法

### 変更対象ファイル

- `src/stats.rs` — `Counter` 型 (新規追加)。doc コメントで将来の `Gauge` 型追加余地を明記
- `src/lib.rs` — `mod stats;` の追加と `DecoderStats` / `EncoderStats` の re-export 追加
- `src/decode.rs` — `DecoderStats` 定義、`DecoderState` に `Arc<Counter>` フィールド追加、`Decoder` 構造体に `Arc<Counter>` フィールド追加、`Decoder::stats()` 追加、`handle_video_sequence_inner` の create 成功後にインクリメント
- `src/encode.rs` — `EncoderStats` 定義、`EncoderState` に `Arc<Counter>` フィールド追加、`Encoder` 構造体に `Arc<Counter>` / `max_in_flight_frames: u32` フィールド追加、`Encoder::stats()` 追加、`encoder buffer is full` 発生箇所でインクリメント
- `README.md` / `skills/shiguredo-nvcodec/SKILL.md` — 統計値取得節の追加 + in-flight 制御のレシピ
- `CHANGES.md` — 追記例:
  - `- [ADD] Decoder::stats() / Encoder::stats() で内部状態 (counter / gauge) を取得できるようにする`
  - `  - @担当者`

## 関連 issue

- 0024: 本 issue の派生元 (0024 の実装検証で追加した `#[cfg(test)]` カウンターが本 API の発想の元)。ただし本 issue は 0024 とは独立して develop ベースで進める。0024 が develop にマージされる際に、0024 側が `#[cfg(test)]` カウンターを削除して本 API に統合し、`reconfigure_decoder_count` / `reconfigure_failure_count` のインクリメントを追加する。なお 0024 の issue の関連 issue 節には「0027 がカウンターを統合する」と記載されているが、独立方針のため実際の統合は 0024 のマージ時作業となり、0024 の issue の該当記述は 0024 実装時に整合させること
- 0026 (closed): `Decoder::reconfigure_failure_count()` の個別 API 案。本 issue に吸収して close 済み
