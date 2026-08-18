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

- [CHANGE] MSRV (rust-version) を 1.93 に上げる
  - @voluntas
- [CHANGE] 一度デコードエラーが起きた Decoder インスタンスは使用不能にし、以降のデコードを行わないようにする
  - 従来はエラー発生後にもデコードを継続することはできたが、同一事象のエラーが二重に通知されたり、ユーザーデータの対応が壊れる経路があった
  - この仕様を変更して、エラー後にフレームのデコードを試みた場合は、常にエラーがコールバックに通知されるようにする
  - エラー発生後の Decoder インスタンスが復旧することはないので、必要なら利用側で Decoder インスタンスを作り直すこと
  - @sile
- [ADD] Decoder::stats() / Encoder::stats() で内部状態 (counter / gauge) を取得できるようにする
  - @sile
- [FIX] Decoder の display area と DecodedFrame の画素領域が一致しない問題を修正する
  - display area の left / top が非ゼロの場合、公開する寸法は表示領域の寸法 (crop 後) なのに、画素データは mapped output surface の左上を起点にコピーされていた
  - コピー元を表示領域の左上に合わせることで、`width()` / `height()` と Y / UV データを表示領域に一致させた
  - `DecodedFrame` の出力契約 (寸法・stride・Y/UV データは表示領域に一致する) を rustdoc に明記した
  - 奇数幅 (width が奇数) のストリームでは、NV12 の UV 行バイト幅を `ceil(width/2)*2` としてコピーするようにした (修正前は最後のクロマ 1 組がコピーされず 0 埋めのままだった)
  - display area の left / top が奇数の入力を、デコード不可の入力として Decoder を終端させるようになった (従来は画素が不整合のままデコードされていた)
  - 原点 0 の解像度変化ストリームによる再作成経路の回帰テストを追加した
  - 奇数幅の JPEG による UV 行バイト幅の回帰テストを追加した
  - 非ゼロ原点 (left / top が非ゼロ) の画素一致は NVIDIA GPU 実機未確認のため、検証待ち
  - @sile
- [FIX] Decoder の parser DPB と内部 decode surface の数が一致しない場合がある問題を修正する
  - decode surface はデコード済みフレームを一時的に格納する GPU 上のバッファで、参照フレームを保持するために複数必要。その数のことをデコードサーフェス数と呼ぶ
  - parser DPB は parser がデコード結果をどのサーフェスへ書き込むかを決めるためのサーフェスの循環リスト
  - `min_num_decode_surfaces == 1` のとき parser の DPB 数が更新されず、parser と decoder のサーフェス数が一致しないことがあった
  - 例えば JPEG は `min_num_decode_surfaces == 1` になり得るため、parser の DPB 数が 1 面以上に保たれたまま decoder が 1 面しか確保しないと、parser が存在しないサーフェスを指す picture index を通知し、`cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` で失敗する可能性があった
  - 両者を同じ実効サーフェス数 (sequence callback の戻り値) で同期するようにした
  - `DecoderConfig.max_num_decode_surfaces` に 0 を指定すると `Decoder::new` が設定エラーとして拒否するようになった (従来は受け付けていた)
  - `min_num_decode_surfaces` が上限を超える場合は、`decode()` 中の sequence callback で既存 decoder を破棄する前にエラーが返るようになった
  - @sile
- [FIX] 10bit 以上の入力をデコード不可として Decoder を終端させる
  - 出力サーフェスは 8bit NV12 のみ対応なのに 10bit ストリーム (`bit_depth_luma_minus8 != 0`) を拒否しておらず、10bit 入力で Y / UV のバイト幅計算が崩れて不正な画素データが返っていた
  - @sile

### misc

- [ADD] CI の CUDA ビルド確認に Ubuntu 26.04（CUDA 13.3.1）を追加する
  - @voluntas

## 2026.2.0

**リリース日**: 2026-06-23

- [CHANGE] エンコード・デコードの結果をトレイトベースのハンドラーを使って非同期で受け取るようにする
  - `EncodeHandler` トレイトと `FnEncodeHandler` ラッパーを追加
  - `DecodeHandler` トレイトと `FnDecodeHandler` ラッパーを追加
  - `Encoder` を `Encoder<H: EncodeHandler>` に、`Decoder` を `Decoder<H: DecodeHandler>` に変更
  - `Encoder::new()` と `Decoder::new()` に完了用コールバックを受け取るハンドラを渡すようにする
  - `Encoder::next_frame()` と `Decoder::next_frame()` は廃止
  - `Encoder::query_caps()` と `Decoder::query_caps()` は `query_encoder_caps()` 及び `query_decoder_caps()` に変更
  - @melpon

## 2026.1.0

**リリース日**: 2026-03-31

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
