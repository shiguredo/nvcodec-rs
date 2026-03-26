# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

- [CHANGE] `hisui/crates/shiguredo_nvcodec/` から `shiguredo/nvcodec-rs` に変更する
  - crates.io はそのまま
  - @voluntas
- [CHANGE] `libloading` クレートへの依存を廃止し、独自の動的ライブラリローダー `dl::DynLib` モジュールに置き換える
  - @voluntas
- [CHANGE] ビルド依存の `toml` クレートを `shiguredo_toml` に置き換える
  - @voluntas
- [CHANGE] `Encoder::new_h264` / `Encoder::new_h265` / `Encoder::new_av1` を廃止し `Encoder::new` に統合する
  - `EncoderConfig` に `codec: CodecConfig` フィールドを追加し、コーデック種別とコーデック固有設定を一体化する
  - @voluntas
- [CHANGE] `Profile` 構造体を廃止し、コーデック固有のプロファイル enum に置き換える
  - `H264Profile` / `HevcProfile` / `Av1Profile` を追加する
  - @voluntas
- [CHANGE] `EncoderConfig` から `profile` と `idr_period` フィールドを削除する
  - コーデック固有設定構造体 (`H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig`) に移動する
  - @voluntas
- [CHANGE] `EncoderConfig` / `H264EncoderConfig` / `HevcEncoderConfig` / `Av1EncoderConfig` / `DecoderConfig` から `Default` 実装を削除する
  - NVENC / NVDEC SDK にデフォルト値の概念がないため、全フィールドを明示的に指定する設計にする
  - @voluntas
- [CHANGE] `Decoder::new_h264` / `Decoder::new_h265` / `Decoder::new_av1` を廃止し `Decoder::new` に統合する
  - `DecoderConfig` に `codec: DecoderCodec` フィールドを追加する
  - @voluntas
- [CHANGE] `Encoder::encode()` に `EncodeOptions` 引数を追加する
  - `force_intra` / `force_idr` / `output_spspps` フラグでフレーム単位のエンコード制御が可能になる
  - @voluntas
- [CHANGE] `EncoderConfig` に `buffer_format: BufferFormat` フィールドを追加する
  - NV12 ハードコードを廃止し、NVENC SDK がサポートする入力バッファフォーマットを選択可能にする
  - @voluntas
- [CHANGE] `DecoderConfig` に `surface_format: SurfaceFormat` フィールドを追加する
  - NV12 ハードコードを廃止し、NVDEC SDK がサポートする出力サーフェスフォーマットを選択可能にする
  - @voluntas
- [ADD] エンコーダーの動的解像度変更に対応する
  - `ReconfigureParams` に `width` / `height` フィールドを追加する
  - `maxEncodeWidth` / `maxEncodeHeight` を超える場合はエラーを返す
  - @voluntas
- [ADD] エンコーダのケーパビリティクエリ機能を追加する
  - `EncoderCaps` 構造体と `Encoder::query_caps` メソッドを追加
  - @voluntas
- [ADD] デコーダのケーパビリティクエリ機能を追加する
  - `DecoderCaps` 構造体と `Decoder::query_caps` メソッドを追加
  - @voluntas
- [ADD] エンコーダの動的再設定機能を追加する
  - `ReconfigureParams` 構造体と `reconfigure` メソッドを追加
  - @voluntas
- [ADD] VP8 / VP9 / JPEG デコーダを追加する
  - `DecoderCodec::Vp8` / `DecoderCodec::Vp9` / `DecoderCodec::Jpeg` を追加
  - @voluntas
- [ADD] CUDA デバイス列挙関数を追加する
  - `device_count()` および `device_name()` 関数を追加
  - @voluntas
- [ADD] CUDA ストリーム管理機能を追加する
  - `CudaStream` 構造体を追加
  - @voluntas
- [ADD] 2D メモリコピー機能を追加する
  - `memcpy_2d()` および `mem_alloc_pitch()` 関数を追加
  - @voluntas
- [FIX] デコーダーがストリーム中の解像度変更に対応できない問題を修正する
  - `handle_video_sequence_inner` で既存デコーダーを破棄して再作成するようにする
  - @voluntas
- [FIX] `DecodedFrame::uv_plane()` が奇数高さの場合に UV プレーンのサイズを 1 行分少なく返す問題を修正する
  - `height / 2` を `height.div_ceil(2)` に変更し、奇数高さでも正しい行数の UV データを返すようにする
  - @sile
- [UPDATE] CUDA インクルードパスの解決をフォールバック付きの 3 段階方式に改善する
  - 環境変数 → デフォルトパス → スタブヘッダの順で探索する
  - @voluntas
- [UPDATE] `EncoderCaps` に `support_yuv422_encode` / `width_min` / `height_min` / `num_max_bframes` / `support_lookahead` / `support_temporal_aq` フィールドを追加する
  - @voluntas
- [ADD] `supported_codecs()` 関数を追加する
  - 指定 GPU デバイスで利用可能なコーデックのエンコード/デコード対応状況を一括で取得する
  - @voluntas
- [UPDATE] docs.rs 向けスタブ生成を包括的な型定義に書き直す
  - @voluntas

## 2025.2.2

- [UPDATE] エラーメッセージを改善する
  - CUDA および NVENC のエラーコードに対応する詳細情報を表示するようにする
  - @sile

## 2025.2.1

**リリース日**: 2025-10-21

- [FIX] ビルドに必要なヘッダファイルを含んだ third_party/ ディレクトリを crate 内に移動する
  - 今までは hisui リポジトリのルートに配置していたが、これだと shiguredo_nvcodec の crates.io への publish 時に third_party/ がパッケージに含まれない
  - そのため cargo 経由でビルドする際に必要なファイルが見つからずに失敗してしまっていた
  - third_party/ ディレクトリを hisui/crates/shiguredo_nvcodec/ 以下に移動することで、crates.io に登録したパッケージにもこのディレクトリが含まれるようにした
  - @sile
