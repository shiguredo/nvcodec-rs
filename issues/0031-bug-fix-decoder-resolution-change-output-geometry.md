# 0031-bug-fix-decoder-resolution-change-output-geometry

- Created: 2026-08-17
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/fix-decoder-resolution-change-output-geometry
- Polished: {YYYY-MM-DD} (例: 2024-07-15)
- Reporter: @sile

## 目的

ストリーム中の解像度変更前後で、`DecodedFrame` の寸法、stride、Y / UV データが、同じフレームの NVDEC 出力サーフェスと一致するようにする。

develop ブランチに存在する問題と、issue 0024 の feature ブランチで追加された reconfigure 固有の問題を区別する。

- develop ブランチでは、`display_area.left` または `display_area.top` が非ゼロの場合に、公開する寸法とコピーする画素領域が一致しない問題を修正する
- develop ブランチの遅延フレームを伴う解像度変更は、現時点では問題が確定していないため、実機で callback の順序と出力を検証する
- issue 0024 の feature ブランチでは、固定した出力サーフェスのジオメトリと reconfigure 後の `DecoderState` が一致しない問題を修正する

`max_display_delay = 0` かつ `display_area.left = 0`、`display_area.top = 0` の通常の decoder 再作成経路には、現時点で出力ジオメトリの問題は確認できていない。

## 現状

### develop ブランチの通常の decoder 再作成経路

develop ブランチでは sequence callback ごとに decoder を再作成し、`CUVIDDECODECREATEINFO.ulTargetWidth` / `ulTargetHeight` と `DecoderState.surface_width` / `surface_height` を、その sequence の coded サイズに更新している。

`max_display_delay = 0` で、`CUVIDEOFORMAT.display_area.left` / `top` がともに 0 の入力では、mapped output surface の高さと UV プレーン開始位置の計算に同じ coded 高さを使っている。

この条件の decoder 再作成経路には、reconfigure 経路で発生する固定 target サイズとの不一致はない。

したがって、develop ブランチの通常経路を、後述する reconfigure 固有の確定問題と同じ問題として扱わない。

### develop ブランチでコード上確認できる問題

`DecoderState.width` / `height` は、`CUVIDEOFORMAT.display_area` の幅と高さから計算している。

一方、develop ブランチの decoder 作成処理は `CUVIDDECODECREATEINFO.display_area` を設定せず、display callback の NV12 コピー処理も `display_area.left` / `top` に対応するコピー元オフセットを適用していない。

このため、`display_area.left` または `display_area.top` が非ゼロの入力では、`DecodedFrame` は crop 後の寸法を公開する一方で、画素データは mapped output surface の左上を起点としてコピーされる。

これは解像度変更の有無や decoder 再作成の有無にかかわらず発生し得る、develop ブランチの条件付きの不具合である。

実際の入力で非ゼロの原点を持つ `display_area` が通知されることと、修正方法が各 codec と GPU / driver で正しく動作することは、NVIDIA GPU 実機で確認する。

### develop ブランチで未確認のリスク

`src/decode.rs` の `DecoderState` は、現在の `width` / `height` / `surface_width` / `surface_height` を 1 組だけ保持している。

`handle_video_sequence_inner` または `DecoderState::handle_video_sequence` は sequence callback のたびにこれらを更新し、`handle_picture_display_inner` または `DecoderState::handle_picture_display` は表示対象の picture に対応する情報ではなく、`DecoderState` の最新値を使って次の値を計算している。

- `DecodedFrame` の `width` / `height`
- Y / UV プレーンのコピー量
- 出力サーフェス上の UV プレーン開始位置

`CUVIDPARSERDISPINFO` は `picture_index` を持つが、解像度や display area を持たない。`max_display_delay > 0` や B フレームを含む入力では、sequence callback 後に以前の sequence の display callback が来た場合、最新のジオメトリを古い picture に適用する可能性がある。

ただし、現時点では sequence callback 後に旧 sequence の display callback が発生することを再現できていない。

NVIDIA の公開仕様では EOS による display 待ち picture の排出は説明されているが、sequence 変更時に同じ排出が必ず完了するとは明記されていない。

既存の解像度変更テストは `max_display_delay = 0` を指定し、sequence 変更時に処理中のフレームが残らないことを前提としているため、この可能性を検証できていない。

したがって、この項目は develop ブランチの確定した不具合ではなく、実機検証が必要なリスクとして扱う。

### issue 0024 の feature ブランチで確認できる問題

issue 0024 の feature ブランチにある reconfigure 実装では、`CUVIDRECONFIGUREDECODERINFO.ulTargetWidth` / `ulTargetHeight` と `display_area` を decoder 作成時の値に固定している。一方、reconfigure 成功後に `DecoderState` の寸法を新しい `CUVIDEOFORMAT` から更新している。

たとえば 320x240 から 256x160 へ変更した場合、NVDEC の出力サーフェスは 320x240 のままでも、UV オフセットを 160 行分の pitch から計算するため、Y データの途中を UV データとしてコピーする可能性がある。

これは固定した target 高さと、UV オフセットの計算に使う高さが一致しないことからコード上確認できる、reconfigure 固有の問題である。

また、現在の feature ブランチで利用側が指定した最大 coded サイズより大きい sequence を reconfigure すると、mapped output surface の範囲外をコピー元として計算する可能性がある。

issue 0024 で検討している「最初の解像度を最大値として作成し、範囲を超えたら decoder を再作成する」設計では、最大値を超える場合の問題は回避できる。

一方、最大値の範囲内で縮小する場合も target サイズと表示対象のジオメトリは異なり得るため、固定 target サイズに合わせたコピー処理、または decoder 再作成へのフォールバックが必要になる。

現在の解像度変更テストは、主にフレーム数、`DecodedFrame` の寸法、create / reconfigure 回数を確認している。既知の画素配置と Y / UV の内容を検証していないため、この不整合を検出できない。

## 影響範囲と優先度

| 対象 | 判定 | 影響 | 対応 |
|---|---|---|---|
| develop ブランチで `display_area.left` または `top` が非ゼロの入力 | コード上確認できる問題 | crop 後の寸法と実際の画素領域が一致せず、利用側が意図しない領域を表示または処理する | issue 0024 と独立した develop ブランチの問題として、実機再現と修正を先行する |
| develop ブランチで `max_display_delay > 0` または B フレームを含む解像度変更 | 未確認のリスク | callback の順序によっては、フレーム欠落、異なる sequence のジオメトリ適用、map / copy の失敗が起こり得る | 問題があると断定せず、実機テストで callback の順序と画素を確認する |
| develop ブランチの通常の decoder 再作成経路 | 問題を確認できていない | `max_display_delay = 0` かつ display area の原点が 0 の範囲では、既知の不整合はない | リグレッションテストの対象とする |
| issue 0024 の feature ブランチにある reconfigure 経路 | コード上確認できる問題 | 色化け、Y データの UV プレーンへの混入、crop / scale の不一致、CUDA map / copy の失敗につながり得る | develop ブランチへのマージ前に修正する |

Rust 側の `Vec` の参照範囲はプレーンサイズで制限されるため、主な実害は GPU 側のコピー元の選択と、利用側へ返す画素内容の不一致である。

ただし、現在の feature ブランチで target サイズを超えるジオメトリからコピー元を計算した場合は、GPU メモリの出力サーフェス範囲外を参照する可能性がある。

## 設計方針

### 対応順序

1. develop ブランチで `display_area.left` または `top` が非ゼロになる入力を用意し、現在のコピー結果を実機で確認する
2. develop ブランチの crop 処理を修正し、解像度が変わらない入力と decoder 再作成を伴う入力の両方で検証する
3. `max_display_delay > 0` と B フレームを含む入力で callback の順序を記録し、旧 sequence の display 待ち picture が残るかを確認する
4. 旧 sequence の picture が残る場合に限り、picture ごとのジオメトリ管理または明示的な排出方法を実装する
5. issue 0024 の reconfigure 経路で、固定 target サイズと `DecodedFrame` の出力契約を両立させる

### `DecodedFrame` の出力契約

`DecodedFrame::width()` / `height()` は、その picture の `CUVIDEOFORMAT.display_area` が表す表示対象の寸法とする。

Y / UV データは同じ picture の表示領域に対応し、次の条件を満たすものとする。

- `y_stride()` / `uv_stride()` が返す stride と各行の配置が一致する
- `y_plane()` が Y データだけを返す
- `uv_plane()` が UV データだけを返す
- coded サイズの padding や display area の crop を画素データへ正しく反映する
- reconfigure の最適化によって、利用側から見える解像度を作成時サイズへ暗黙に固定しない

この契約を reconfigure で維持できない解像度変更は、decoder の再作成経路で処理する。

### develop ブランチの crop 処理

`display_area.left` / `top` が非ゼロの場合も、`DecodedFrame::width()` / `height()` と Y / UV のコピー元領域を一致させる。

次の方式を実機で比較し、codec と GPU / driver に依存せず出力契約を維持できる方式を採用する。

1. `CUVIDDECODECREATEINFO.display_area` と target rect を設定し、mapped output surface を表示領域に合わせる
2. coded サイズの mapped output surface から、Y / UV の行ごとに display area のオフセットを適用してコピーする

NV12 の UV プレーンは 2x2 のクロマサブサンプリングを使うため、display area の座標制約と UV のコピー元オフセットも確認する。

### 遅延フレームの検証とジオメトリ管理

sequence callback で更新した最新値を、すべての display callback に無条件で使用しない。

ただし、最初から picture ごとの状態管理が必要だとは決めない。

まず、sequence callback の前に旧 sequence の display 待ち picture が排出されるかを実機で検証する。

旧 sequence の picture が残る場合は、`CUVIDPICPARAMS.CurrPicIdx` と `CUVIDPARSERDISPINFO.picture_index` を利用して picture ごとのジオメトリを関連付ける方法、または sequence 変更前の display 待ち picture を完了させてから状態を更新する方法を検討する。

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

- 解像度を変更せず、`display_area.left` または `top` が非ゼロの入力
- `display_area.left` または `top` が非ゼロで、decoder 再作成を伴う解像度変更
- 320x240 → 256x160 → 320x240 の縮小と復帰
- 256x160 → 320x240 → 256x160 の拡大と縮小
- `max_display_delay = 0`
- `max_display_delay > 0` かつ B フレームを含む H.264 または H.265
- coded サイズと display area の幅、高さ、原点のいずれかが異なる入力
- develop ブランチの decoder 再作成経路
- issue 0024 の feature ブランチにある `cuvidReconfigureDecoder` 経路

各出力フレームについて次を確認する。

- フレーム順序とフレーム数
- `width()` / `height()`
- Y / UV の stride とプレーンサイズ
- 既知の座標にある Y / U / V の値
- `display_area` の左上と右下に対応する画素が正しいこと
- 解像度変更境界で、以前の sequence の画素や Y データが UV プレーンへ混入していないこと
- sequence callback と decode / display callback の順序

## 完了条件

- `DecodedFrame` の寸法と画素データに関する公開契約が rustdoc に明記されている
- develop ブランチで `display_area.left` または `top` が非ゼロの場合も、寸法、stride、Y / UV データが表示領域と一致する
- develop ブランチの通常の decoder 再作成経路にリグレッションがない
- `max_display_delay > 0` と B フレームを含む解像度変更について、callback の順序と旧 sequence の display 待ち picture の有無が実機で確認されている
- 旧 sequence の display 待ち picture が残る場合は、display callback がその picture と無関係な最新ジオメトリを使用しない
- 旧 sequence の display 待ち picture が残らない場合は、その前提と検証結果がコードコメントまたはテストに記録されている
- issue 0024 の reconfigure 経路で、寸法、stride、Y / UV データが一致する
- reconfigure で出力契約を維持できない条件が、decoder 再作成へフォールバックする条件として明文化されている
- パターン映像を使った NVIDIA GPU 実機テストが追加されている
- issue 0024 の出力ジオメトリに関する完了条件を満たせる状態になっている
- `CHANGES.md` の `develop` セクションに `[FIX]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — display area に対応する NV12 コピー処理、遅延フレームの検証に応じたジオメトリ管理、reconfigure 適用判定、実機テストを修正する
- `testdata/resolution-change/` — segment と Y / UV の位置を識別できるパターン映像、display area の原点が非ゼロの入力、および B フレームを含む入力を追加する
- `README.md` — 動的解像度変更時の `DecodedFrame` の出力契約を説明する
- `skills/shiguredo-nvcodec/SKILL.md` — 動的解像度変更と出力ジオメトリの説明を更新する
- `CHANGES.md` — 次の `[FIX]` エントリを追加する

```markdown
- [FIX] Decoder の display area と DecodedFrame の画素領域が一致しない問題を修正する
  - @sile
```

issue 0024 の未マージの reconfigure 実装だけに存在する問題は、独立した `[FIX]` エントリにはせず、issue 0024 の変更内容へ含める。

## 関連 issue

- 0006 (closed): decoder 再作成による動的解像度変更を導入した。本 issue では通常の再作成経路をリグレッション対象とし、遅延フレームが残る場合だけ picture ごとの寸法管理を修正する
- 0024 (open): reconfigure と decoder 再作成のハイブリッド化。本 issue の完了を reconfigure 実装の前提とする
