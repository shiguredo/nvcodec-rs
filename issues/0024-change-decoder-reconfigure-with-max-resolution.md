# 0024-change-decoder-reconfigure-with-max-resolution

- Created: 2026-08-05
- Branch: feature/change-decoder-reconfigure-with-max-resolution
- Updated: 2026-08-14

## 目的

Sora の WebRTC シミュキャスト録画のように 1 つの MP4 内でシーケンスヘッダ (SPS/PPS/VPS 等) が変わるたびに符号化解像度が変わるストリームで、シーケンス変更ごとに `cuvidCreateDecoder` を呼び直すコストを削減する。

現行の destroy+create 方式ではシーケンス変更ごとに NVDEC デコーダーの作成コストが乗り、デコード全体の処理時間が伸びる。NVDEC SDK が想定している `cuvidReconfigureDecoder` による in-place 再構成に切り替えて回避する。

## 現状

`handle_video_sequence_inner` (`src/decode.rs`) は `pfnSequenceCallback` で解像度変更を検出すると `cuvidDestroyDecoder` で既存デコーダーを破棄してから `cuvidCreateDecoder` で新規作成する (issue 0006 で「方法 1: デコーダーの再作成」として採用)。

このため `CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` は毎回そのシーケンスの `format.coded_width` / `coded_height` に上書きされ、`cuvidReconfigureDecoder` 用途としては機能していない。

issue 0006 では「方法 2: `cuvidReconfigureDecoder` を使用」も検討されたが、`ulMaxWidth` / `ulMaxHeight` を事前に知ることが難しいという理由で見送られた。

## 設計方針

呼び出し側が最大解像度を知っている場合に限り、`cuvidReconfigureDecoder` による in-place 再構成に切り替える。知らない場合は現状どおりの destroy+create にフォールバックする。

### API 追加とカテゴリ

`DecoderConfig` に以下の pub フィールドを追加する。

- `max_coded_width: Option<u32>`
- `max_coded_height: Option<u32>`

`DecoderConfig` は `#[derive(Debug, Clone)]` の pub struct で `Default` 実装を持たない (2026.1.0 で明示的に削除済み) ため、pub フィールドの追加は既存 struct literal 初期化コードを壊す **破壊的変更 (`[CHANGE]`)** に該当する。issue のタイトル prefix・Branch prefix・CHANGES.md エントリの分類はいずれも `change` に統一する。

### encoder 側 (`max_encode_width` / `max_encode_height`) との命名と意味論の違い

命名は SDK 側フィールドに寄せて非対称にする (encoder = `maxEncodeWidth`、decoder = `coded_width`)。意味論も次のように異なる。

- encoder: `None` 時は `width` と同じ値で `maxEncodeWidth` を確定させる (常に `reconfigure()` 可能)
- decoder: `None` 時は現状の destroy+create にフォールバック (reconfigure 経路を使わない)

encoder は明示的に `reconfigure()` を呼び出す API、decoder は `pfnSequenceCallback` で自動追従する API という設計差から来る意図的な非対称であり、命名と意味論のどちらも揃えない。

### `cuvidReconfigureDecoder` の適用条件

`cuvidReconfigureDecoder` は SDK コメント上「for same codec」に限定される (`third_party/nvcodec/include/cuviddec.h` の `cuvidReconfigureDecoder` doc)。したがって以下の条件のいずれかを満たす場合は reconfigure ではなく destroy+create にフォールバックする。

- `state.decoder` が `null` (初回コールバック。この場合は「フォールバック」ではなく「初回作成」)
- `max_coded_width` / `max_coded_height` のいずれかが `None`
- 直前 create/reconfigure 時に保存したコーデック情報 (reconfigure 適用可否判定用のベースライン) から `codec` / `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` / `progressive_sequence` のいずれかが変化した

判定用ベースラインは `DecoderState` に新規フィールドとして保存する。**Step 5 (下記) で codec / chroma / bit_depth / progressive の変化により destroy+create でデコーダーを作り直したときは、保存値も新しい `CUVIDEOFORMAT` の値で更新する** (更新しないと以降のコールバックで永久に destroy+create が続き reconfigure 経路に戻れなくなる)。Step 4 (`max_coded_*` = `None`) の destroy+create フォールバック経路では保存値は使われないため更新不要。

### `handle_video_sequence_inner` の分岐

`pfnSequenceCallback` (`handle_video_sequence_inner`) は以下の順で処理する。max 超過事前検証 (Step 2) は SDK 呼び出し (Step 3〜6) より前に置く。

1. `format.display_area` の負値・境界を検証する (現行の「create → validate」順を「validate → create/reconfigure」順に修正。issue 0017 の「問題 2: `display_area` 検証位置」を destroy+create 経路も含めて解消する)
2. **max 超過事前検証**: `max_coded_width` / `max_coded_height` が両方 `Some` かつ `format.coded_width > max_coded_width` または `format.coded_height > max_coded_height`: エラーを返す (SDK 呼び出し前。初回コールバックか 2 回目以降かによらず検証する)
3. `state.decoder` が `null` (初回作成): `CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` に `max_coded_width` / `max_coded_height` を渡して `cuvidCreateDecoder`。`None` の場合は現状どおり `format.coded_width` / `coded_height` を渡す。判定用ベースラインを `DecoderState` に保存する
4. `state.decoder` が非 `null` かつ `max_coded_width` / `max_coded_height` のいずれかが `None`: **destroy+create フォールバック** (現状動作維持。`ulMaxWidth` / `ulMaxHeight` は `format.coded_width` / `coded_height`。この経路では判定用ベースラインは使わないので保存値更新も不要)
5. `state.decoder` が非 `null` かつ `max_coded_*` が両方 `Some` かつ判定用ベースラインから `codec` / `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` / `progressive_sequence` のいずれかが変化: **destroy+create フォールバック** (`ulMaxWidth` / `ulMaxHeight` には引き続き `max_coded_width` / `max_coded_height` を渡し、新しいコーデック情報で判定用ベースラインを更新して次回以降 reconfigure 経路に戻れるようにする)
6. それ以外: **`cuvidReconfigureDecoder`** で in-place 再構成

reconfigure / destroy+create のいずれのパスでも、成功後は現行 create パスと同じロジックで `state.width` / `state.height` / `state.surface_width` / `state.surface_height` を更新し、戻り値も同じく `Ok(format.min_num_decode_surfaces as i32)` を返す。

### `CUVIDRECONFIGUREDECODERINFO` の設定値

`cuvidReconfigureDecoder` に渡す `CUVIDRECONFIGUREDECODERINFO` は以下のとおり。

- `ulWidth` / `ulHeight` = `format.coded_width` / `coded_height`
- `ulTargetWidth` / `ulTargetHeight` = 初回 `cuvidCreateDecoder` 時の値 (`state.create_geometry` で保持)
  - 縮小方向の解像度変更で新 coded サイズに下げると、既に allocate 済みの出力サーフェスとの不整合により `cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` を返すため、NVIDIA 公式サンプル `NvDecoder::ReconfigureDecoder` と同様に作成時サイズを維持する
- `ulNumDecodeSurfaces` = `format.min_num_decode_surfaces` (本 issue のスコープ内では現行と同じ扱い。codec 別推奨値への引き上げは 0028 で対応)
- `display_area` = 初回 `cuvidCreateDecoder` 時の値 (`state.create_geometry` で保持)
- `target_rect` = ゼロ埋め (`std::mem::zeroed()` で構造体全体を 0 初期化するのに任せる)

Create 側も同様に `CUVIDDECODECREATEINFO.display_area` に `format.display_area` を明示設定する (以降の `cuvidReconfigureDecoder` で同じ値を再度渡す必要があるため)。

Step 1 で `format.display_area` を検証するのは、`state.width` / `state.height` の計算に使う `right - left` などが破綻しないことを保証するのが目的。

`format.coded_width` / `coded_height` は `u32` だが `CUVIDRECONFIGUREDECODERINFO.ulWidth` / `ulHeight` は `unsigned int` (bindgen 生成後は `c_uint`) なので通常のキャストで問題ない。`display_area` は i32 → i16 のキャストになるが、`validate_display_area` で負値・逆転・coded 超過を弾いており、実用上の解像度は i16 の上限を超えないため安全。

### 失敗時の状態遷移

`handle_video_sequence_inner` が `Err` を返す経路は、いずれも実装済みの終端契約に入る。`handle_video_sequence` は `frame_tx.send(Err(...))` しない。`callback_error` に最初の 1 件だけ格納し、パーサーには失敗 (`0`) を返す。`DecoderState::decode` がその slot を優先して `Err` を返し、`DecodeWorker::run` が原因 `Err` を 1 回通知して終端する。以降 `DecoderState::decode` は呼ばない。復旧は `Decoder` を作り直す。

各経路で `state.decoder` がどう残るかは次のとおり。終端後は使わないので、古い解像度での継続や次回コールバックでの再試行はしない。

- `display_area` 検証失敗 (Step 1): SDK 呼び出しなしのため `state.decoder` は前デコーダー (あるいは初回コールバックなら null) のまま残る。**現行実装は「create → validate」順で失敗時に古いデコーダーが破棄済み状態で Err を返していた (issue 0017 問題 2)**。本 issue の Step 1 変更でこの半壊状態を回避する
- max 超過事前検証エラー (Step 2): SDK 呼び出しなしのため `state.decoder` は前デコーダー (あるいは初回コールバックなら null) のまま残る
- `cuvidCreateDecoder` 失敗 (Step 3/4/5 経路): `state.decoder` は null になる。当該インスタンスは終端するため、0017 の「問題 1: 順序」による継続不能は公開契約上は現れない。destroy 済みで create に失敗した半壊は、作り直しで捨てる
- `cuvidReconfigureDecoder` 失敗 (Step 6 経路): NVDEC SDK は失敗後のデコーダー状態を明示していない。安全側に倒し、`state.decoder` は変更しない。当該インスタンスは終端するため、次の解像度変化で再試行しない

### `CudaLibrary` への追加

`src/lib.rs` の `CudaLibrary::load` は全 nvcuvid 関数を `nvcuvid_lib.get(...)` で存在チェックしている。同じパターンで以下 2 点を追加する。

- `CudaLibrary::load` 内 `cuvidDestroyDecoder` の存在チェック近傍に `cuvidReconfigureDecoder` の存在チェックを追加
- `CudaLibrary::cuvid_reconfigure_decoder(&self, decoder, params) -> Result<(), Error>` メソッドを `cuvid_create_decoder` / `cuvid_destroy_decoder` の隣に追加。型は bindgen 生成の `sys::CUVIDRECONFIGUREDECODERINFO` を使う

## 完了条件

- `DecoderConfig` に `max_coded_width: Option<u32>` / `max_coded_height: Option<u32>` が追加され、既存の struct literal 初期化コード (`test_decoder_config`、`README.md`、`skills/shiguredo-nvcodec/SKILL.md` のコード例) がすべて明示的に更新されている
- `Some(v)` を渡し、解像度のみが変化するストリームで、`pfnSequenceCallback` の 2 回目以降で `cuvidReconfigureDecoder` が呼ばれ `cuvidCreateDecoder` は呼ばれない挙動が確認できる (`Decoder::stats()` の `total_create_decoder_count` が 1 で `total_reconfigure_decoder_count` が 1 以上になることでも確認する)
- `Some(v)` を渡し、codec / chroma / bit depth / progressive のいずれかが変化した場合に destroy+create パスにフォールバックし、以降 reconfigure 経路に戻れる挙動が確認できる
- `Some(v)` を渡し、`coded_width` / `coded_height` が `v` を超えたときに原因 `Err` が 1 回通知され、当該 `Decoder` が終端することが確認できる (初回コールバック / 2 回目以降のいずれのケースでも。0029 の終端契約テストは下記メモ)
- `None` を渡した場合、2026.2.0 と同じ動作 (シーケンス変更ごとに destroy+create) を維持する
- `display_area` 検証位置を先頭に移した結果、destroy+create 経路でも invalid `display_area` で失敗した場合に古いデコーダーが破棄されないことが確認できる
- `CHANGES.md` に `[CHANGE]` エントリが追加されている
- `README.md` と `skills/shiguredo-nvcodec/SKILL.md` の「動的解像度変更」節 (デコーダー / まとめ表) および `DecoderConfig` 表・コード例が新 API を反映している

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — `DecoderConfig` フィールド追加、`DecoderState` に判定用ベースライン (コーデック情報) 保存フィールド追加、`DecoderState` に `create_geometry` (`DecoderCreateGeometry`) フィールド追加 (`cuvidCreateDecoder` 成功時に `target_width` / `target_height` / `display_area` を保存し、以降の `cuvidReconfigureDecoder` で参照する)、`handle_video_sequence_inner` の分岐再構成 (`display_area` 検証・max 超過事前検証を先頭に移動、reconfigure / destroy+create の 6 ステップ分岐)、既存の struct literal 初期化コード (`test_decoder_config` 等) の更新、reconfigure 成功時に `DecoderStats::total_reconfigure_decoder_count` をインクリメント
- `src/lib.rs` — `CudaLibrary::load` に `cuvidReconfigureDecoder` の存在チェック追加、`cuvid_reconfigure_decoder` ラッパー追加
- `testdata/resolution-change/{h264.h264, h265.h265, vp8.ivf, vp9.ivf, av1.ivf}` — 320x240 x15 + 256x160 x15 + 320x240 x15 の 3 セグメント構成で新規追加 (HEVC 144x144 / VP9 / AV1 128x128 の hardware 最小デコード解像度を上回るサイズ)
- `testdata/resolution-change/README.md` — テストデータの構造と解像度選定理由を記載
- `README.md` — 「デコード」コード例の `DecoderConfig` struct literal に `max_coded_width` / `max_coded_height` を追記 (`None` を渡し従来動作を示す)
- `skills/shiguredo-nvcodec/SKILL.md` — 「動的解像度変更」節 (デコーダー / まとめ表) と `DecoderConfig` 表に `max_coded_width` / `max_coded_height` を追記。デコーダーのコード例の `DecoderConfig` struct literal にも同フィールドを追記
- `CHANGES.md` — 追記例:
  - `- [CHANGE] DecoderConfig に max_coded_width / max_coded_height を追加してデコーダーの動的解像度変更を cuvidReconfigureDecoder で行えるようにする`
  - `  - @担当者`

## 実装で判明した追加事項

実装検証で以下 2 点が判明した。1 点目は本 issue の設計方針節に反映済み。

- **`ulTargetWidth` / `ulTargetHeight` と `display_area` は作成時サイズに固定する**: 縮小方向 reconfigure 直後の `cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` を返す事例があり、NVIDIA 公式サンプル準拠で作成時サイズを保持する必要があると判明。詳細は上記「`CUVIDRECONFIGUREDECODERINFO` の設定値」節を参照
- **テストデータの解像度制約**: `query_decoder_caps` で得られる各コーデックの hardware 最小デコード解像度 (HEVC: 144x144、VP9 / AV1: 128x128) を下回るテストデータでは、シーケンスコールバックは正常に発火して decoder 作成 / reconfigure も成功するが、その解像度での `cuvidDecodePicture` が全ピクチャで `CUDA_ERROR_INVALID_VALUE` を返す (reconfigure / destroy+create のどちらの経路でも同じ)。テストデータは全 codec の min を上回る解像度で生成する (本 issue では 256x160 を採用)

### 派生的な検討事項として切り出した別 issue

本 issue の実装検証で派生的に浮上した検討事項は、意味論を独立議論するために別 issue として起票済み。以下 3 件のうち、0027 と 0029 は develop にマージ済み (closed) で、0028 のみ pending のまま残っている。

- **0027 (closed)**: `Decoder` / `Encoder` 統計値 API を追加する — 実装検証時の `#[cfg(test)]` カウンター (`create_decoder_count` / `reconfigure_decoder_count`) を pub 化して統一 API に統合する。develop マージ済みで `DecoderStats` に `total_reconfigure_decoder_count` / `total_reconfigure_failure_count` が既に存在する。本 issue は `#[cfg(test)]` カウンターを追加せず、この統計値 API で reconfigure 呼び出しを検証し、成功時に `total_reconfigure_decoder_count` をインクリメントする
- **0028 (pending)**: `ulNumDecodeSurfaces` を codec 別推奨値に引き上げる — 参照フレーム数の多い HEVC / VP9 / AV1 で DPB 不足リスクを低減する。本 issue のスコープ外であり、`ulNumDecodeSurfaces` は `format.min_num_decode_surfaces` のままとする
- **0029 (closed)**: デコードエラー後の `Decoder` を終端状態にする — 二重通知と `drain_frames` scorched-earth を、エラー後継続をやめることで解消する。develop マージ済みで終端契約は実装済み

### 終端契約テスト（本 issue 実装時に一緒にやる）

0029 (closed) の完了条件にある終端契約テスト（エラー後に Ok が来ない・原因 Err は 1 回・後続ジョブに終端 Err・終端後 `flush` が戻る）は、0029 単体の実装時点では公開 API だけで安定してデコードエラーを起こす手段がなかったため未着手となり、本 issue に後回しになっている (0029 の完了条件に明記済み)。

本 issue の `max_coded_width` / `max_coded_height` 超過による事前検証エラーが、公開 API で安定再現できる Err 誘発手段になる。reconfigure / max 超過のテストを書くタイミングで、上記の終端契約テストも同じ経路で追加する。

## 関連 issue

- 0006 (closed)
- 0017 (pending): destroy-then-create 順序による復旧不能問題。本 issue マージ後の扱い:
  - 「問題 2: `display_area` 検証位置」は本 issue の Step 1 で destroy+create 経路も含めて解消される
  - 「問題 1: 順序」は依然として `max_coded_*` = `None` のフォールバック経路に残るため、0017 は pending を維持する
- 0027 (closed): `Decoder` / `Encoder` 統計値 API 追加 (本 issue の実装検証から派生。develop にマージ済みで、本 issue は `total_reconfigure_decoder_count` / `total_reconfigure_failure_count` のインクリメントを追加する)
- 0028 (pending): `ulNumDecodeSurfaces` codec 別推奨値化 (本 issue の実装検証から派生。本 issue のスコープ外のため pending のまま)
- 0029 (closed): デコードエラー後の Decoder 終端 (本 issue の実装検証から派生。develop にマージ済みで、終端契約テストのみ本 issue に後回し)
