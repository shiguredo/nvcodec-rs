# 0031-bug-fix-decoder-resolution-change-output-geometry

- Created: 2026-08-17
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/fix-decoder-resolution-change-output-geometry
- Polished: {YYYY-MM-DD} (例: 2024-07-15)
- Reporter: @sile

## 目的

ストリーム中の解像度変更前後で、`DecodedFrame` の寸法、stride、Y / UV データが、同じフレームの NVDEC 出力サーフェスと一致するようにする。

decoder の再作成経路と `cuvidReconfigureDecoder` 経路のどちらでも、利用側から見える出力契約を維持する。

## 現状

`src/decode.rs` の `DecoderState` は、現在の `width` / `height` / `surface_width` / `surface_height` を 1 組だけ保持している。

`handle_video_sequence_inner` または `DecoderState::handle_video_sequence` は sequence callback のたびにこれらを更新し、`handle_picture_display_inner` または `DecoderState::handle_picture_display` は表示対象の picture に対応する情報ではなく、`DecoderState` の最新値を使って次の値を計算している。

- `DecodedFrame` の `width` / `height`
- Y / UV プレーンのコピー量
- 出力サーフェス上の UV プレーン開始位置

`CUVIDPARSERDISPINFO` は `picture_index` を持つが、解像度や display area を持たない。`max_display_delay > 0` や B フレームを含む入力では、sequence callback 後に以前の sequence の display callback が来た場合、最新のジオメトリを古い picture に適用する可能性がある。

issue 0024 の feature ブランチにある reconfigure 実装では、`CUVIDRECONFIGUREDECODERINFO.ulTargetWidth` / `ulTargetHeight` と `display_area` を decoder 作成時の値に固定している。一方、reconfigure 成功後に `DecoderState` の寸法を新しい `CUVIDEOFORMAT` から更新している。

たとえば 320x240 から 256x160 へ変更した場合、NVDEC の出力サーフェスは 320x240 のままでも、UV オフセットを 160 行分の pitch から計算するため、Y データの途中を UV データとしてコピーする可能性がある。

現在の解像度変更テストは、主にフレーム数、`DecodedFrame` の寸法、create / reconfigure 回数を確認している。既知の画素配置と Y / UV の内容を検証していないため、この不整合を検出できない。

## 設計方針

### `DecodedFrame` の出力契約

`DecodedFrame::width()` / `height()` は、その picture の `CUVIDEOFORMAT.display_area` が表す表示対象の寸法とする。

Y / UV データは同じ picture の表示領域に対応し、次の条件を満たすものとする。

- `y_stride()` / `uv_stride()` が返す stride と各行の配置が一致する
- `y_plane()` が Y データだけを返す
- `uv_plane()` が UV データだけを返す
- coded サイズの padding や display area の crop を画素データへ正しく反映する
- reconfigure の最適化によって、利用側から見える解像度を作成時サイズへ暗黙に固定しない

この契約を reconfigure で維持できない解像度変更は、decoder の再作成経路で処理する。

### picture ごとのジオメトリ管理

sequence callback で更新した最新値を、すべての display callback に無条件で使用しない。

`CUVIDPICPARAMS.CurrPicIdx` と `CUVIDPARSERDISPINFO.picture_index` を利用して picture ごとのジオメトリを関連付ける方法、または sequence 変更前の display 待ち picture を完了させてから状態を更新する方法を実機で検証する。

picture index を利用する場合は、次を管理する。

- 表示幅と表示高さ
- mapped output surface の高さ
- display area
- decoder session の世代
- picture の表示完了後に情報を破棄するライフサイクル

### reconfigure 時の出力サーフェス

実機で次の設定を比較する。

1. 新しい coded サイズと display area に合わせて target サイズも更新する
2. NVIDIA 公式サンプルと同様に、作成時の target サイズと display area を維持する

各 codec と GPU / driver で、`cuvidReconfigureDecoder`、後続の `cuvidDecodePicture`、`cuvidMapVideoFrame` が成功するかを確認する。

作成時サイズを維持する方式しか安全に動作しない場合でも、実際の出力サーフェスと異なる寸法を `DecodedFrame` に設定してはならない。既存の出力契約を維持できない場合は、issue 0024 の reconfigure 適用条件から除外して decoder を再作成する。

## テスト戦略

モックやスタブは使わず、NVIDIA GPU 実機で検証する。

`testdata/resolution-change/` に、解像度と segment を画素値から識別できるパターン映像を追加する。単色の黒フレームだけにはしない。

少なくとも次を検証する。

- 320x240 → 256x160 → 320x240 の縮小と復帰
- 256x160 → 320x240 → 256x160 の拡大と縮小
- `max_display_delay = 0`
- `max_display_delay > 0` かつ B フレームを含む H.264 または H.265
- coded サイズと display area が異なる入力
- decoder 再作成経路
- `cuvidReconfigureDecoder` 経路

各出力フレームについて次を確認する。

- フレーム順序とフレーム数
- `width()` / `height()`
- Y / UV の stride とプレーンサイズ
- 既知の座標にある Y / U / V の値
- 解像度変更境界で、以前の sequence の画素や Y データが UV プレーンへ混入していないこと

## 完了条件

- `DecodedFrame` の寸法と画素データに関する公開契約が rustdoc に明記されている
- display callback が、その picture と無関係な最新ジオメトリを使用しない
- decoder 再作成経路と reconfigure 経路の双方で、寸法、stride、Y / UV データが一致する
- `max_display_delay > 0` と B フレームを含む解像度変更で、旧 sequence と新 sequence のジオメトリが混在しない
- reconfigure で出力契約を維持できない条件が、decoder 再作成へフォールバックする条件として明文化されている
- パターン映像を使った NVIDIA GPU 実機テストが追加されている
- issue 0024 の出力ジオメトリに関する完了条件を満たせる状態になっている
- `CHANGES.md` の `develop` セクションに `[FIX]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — picture ごとの出力ジオメトリ管理、reconfigure 適用判定、NV12 コピー処理、実機テストを修正する
- `testdata/resolution-change/` — segment と Y / UV の位置を識別できるパターン映像、および B フレームを含む入力を追加する
- `README.md` — 動的解像度変更時の `DecodedFrame` の出力契約を説明する
- `skills/shiguredo-nvcodec/SKILL.md` — 動的解像度変更と出力ジオメトリの説明を更新する
- `CHANGES.md` — 次の `[FIX]` エントリを追加する

```markdown
- [FIX] 解像度変更時に DecodedFrame のジオメトリと Y / UV データが一致しない問題を修正する
  - @sile
```

## 関連 issue

- 0006 (closed): decoder 再作成による動的解像度変更を導入した。本 issue は picture ごとの寸法が最新の `DecoderState` だけで決まるという前提を再検証する
- 0024 (open): reconfigure と decoder 再作成のハイブリッド化。本 issue の完了を reconfigure 実装の前提とする
