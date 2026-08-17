# 0031-bug-fix-decoder-output-geometry

- Created: 2026-08-17
- Completed: {YYYY-MM-DD} (例: 2024-07-01)
- Branch: feature/fix-decoder-output-geometry
- Polished: {YYYY-MM-DD} (例: 2024-07-15)
- Reporter: @sile

## 目的

develop ブランチの decoder 出力で、`DecodedFrame` の寸法、stride、Y / UV データが、同じフレームの表示領域と一致するようにする。

本 issue では次の 2 点を扱う。

- `CUVIDEOFORMAT.display_area.left` または `top` が非ゼロの場合に、公開する寸法とコピーする画素領域が一致しない問題を修正する
- 遅延フレームを伴う sequence 変更で、旧 sequence の picture に新しいジオメトリを適用する可能性を実機で検証し、必要な場合だけ修正する

issue 0024 の feature ブランチにある reconfigure 固有の target surface と出力ジオメトリの不一致は、本 issue では扱わない。

本 issue で `DecodedFrame` の出力契約と develop の実装を確定し、issue 0024 はその契約を reconfigure 経路でも満たす。

## 現状

### develop ブランチでコード上確認できる問題

`src/decode.rs` の `handle_video_sequence_inner` は、`DecoderState.width` / `height` を `CUVIDEOFORMAT.display_area` の幅と高さから計算している。

一方、decoder 作成処理は `CUVIDDECODECREATEINFO.display_area` を設定せず、`handle_picture_display_inner` の NV12 コピー処理も `display_area.left` / `top` に対応するコピー元オフセットを適用していない。

このため、`display_area.left` または `display_area.top` が非ゼロの入力では、`DecodedFrame` は crop 後の寸法を公開する一方で、画素データは mapped output surface の左上を起点としてコピーされる。

これは解像度変更や decoder 再作成の有無にかかわらず発生し得る、develop ブランチの条件付きの不具合である。

利用側では、公開された寸法に対応しない領域を表示、変換、解析する可能性がある。

実際の入力で非ゼロの原点を持つ `display_area` が通知されることと、修正方法が各 codec と GPU / driver で正しく動作することは、NVIDIA GPU 実機で確認する。

### develop ブランチで未確認のリスク

`DecoderState` は、現在の `width` / `height` / `surface_width` / `surface_height` を 1 組だけ保持している。

`handle_video_sequence_inner` は sequence callback のたびにこれらを更新し、`handle_picture_display_inner` は表示対象の picture に対応する情報ではなく、`DecoderState` の最新値を使って次の値を計算している。

- `DecodedFrame` の `width` / `height`
- Y / UV プレーンのコピー量
- 出力サーフェス上の UV プレーン開始位置

`CUVIDPARSERDISPINFO` は `picture_index` を持つが、解像度や display area を持たない。

`max_display_delay > 0` や B フレームを含む入力では、sequence callback 後に以前の sequence の display callback が来た場合、最新のジオメトリを古い picture に適用する可能性がある。

その場合は、フレーム欠落、異なる sequence のジオメトリ適用、map / copy の失敗につながり得る。

ただし、現時点では sequence callback 後に旧 sequence の display callback が発生することを再現できていない。

NVIDIA の公開仕様では EOS による display 待ち picture の排出は説明されているが、sequence 変更時に同じ排出が必ず完了するとは明記されていない。

既存の解像度変更テストは `max_display_delay = 0` を指定し、sequence 変更時に処理中のフレームが残らないことを前提としているため、この可能性を検証できていない。

したがって、この項目は develop ブランチの確定した不具合ではなく、実機検証が必要なリスクとして扱う。

### develop ブランチで問題が確認できない経路

develop ブランチでは sequence callback ごとに decoder を再作成し、`CUVIDDECODECREATEINFO.ulTargetWidth` / `ulTargetHeight` と `DecoderState.surface_width` / `surface_height` を、その sequence の coded サイズに更新している。

`max_display_delay = 0` で、`display_area.left` / `top` がともに 0 の入力では、mapped output surface の高さと UV プレーン開始位置の計算に同じ coded 高さを使っている。

この条件の通常の decoder 再作成経路には、現時点で出力ジオメトリの問題は確認できていない。

通常経路は修正対象ではなく、リグレッションテストの対象とする。

## 影響範囲と優先度

| 対象 | 判定 | 影響 | 対応 |
|---|---|---|---|
| `display_area.left` または `top` が非ゼロの入力 | コード上確認できる問題 | crop 後の寸法と画素領域が一致せず、意図しない領域を利用側へ返す | develop ブランチの問題として先に修正する |
| `max_display_delay > 0` または B フレームを含む sequence 変更 | 未確認のリスク | callback の順序によっては、旧 sequence の picture に新しいジオメトリを適用する | 実機で callback の順序と画素を確認し、必要な場合だけ修正する |
| `max_display_delay = 0` かつ display area の原点が 0 の通常経路 | 問題を確認できていない | 既知の不整合はない | リグレッションテストの対象とする |

## 設計方針

### `DecodedFrame` の出力契約

`DecodedFrame::width()` / `height()` は、その picture の `CUVIDEOFORMAT.display_area` が表す表示対象の寸法とする。

Y / UV データは同じ picture の表示領域に対応し、次の条件を満たすものとする。

- `y_stride()` / `uv_stride()` が返す stride と各行の配置が一致する
- `y_plane()` が Y データだけを返す
- `uv_plane()` が UV データだけを返す
- coded サイズの padding や display area の crop を画素データへ正しく反映する

この契約を rustdoc に明記する。

issue 0024 の reconfigure 経路も同じ契約を維持するが、その実装と適用条件は issue 0024 で扱う。

### develop ブランチの crop 処理

`display_area.left` / `top` が非ゼロの場合も、`DecodedFrame::width()` / `height()` と Y / UV のコピー元領域を一致させる。

次の方式を実機で比較し、codec と GPU / driver に依存せず出力契約を維持できる方式を採用する。

1. `CUVIDDECODECREATEINFO.display_area` と target rect を設定し、mapped output surface を表示領域に合わせる
2. coded サイズの mapped output surface から、Y / UV の行ごとに display area のオフセットを適用してコピーする

NV12 の UV プレーンは 2x2 のクロマサブサンプリングを使うため、display area の座標制約と UV のコピー元オフセットも確認する。

### 遅延フレームの検証とジオメトリ管理

最初から picture ごとの状態管理が必要だとは決めない。

まず、sequence callback の前に旧 sequence の display 待ち picture が排出されるかを NVIDIA GPU 実機で検証する。

旧 sequence の picture が残らない場合は、その前提と検証結果をコードコメントまたはテストに記録する。

旧 sequence の picture が残る場合は、sequence callback で更新した最新値を、すべての display callback に無条件で使用しない。

次のいずれかを実機で検証して採用する。

1. `CUVIDPICPARAMS.CurrPicIdx` と `CUVIDPARSERDISPINFO.picture_index` を利用して picture ごとのジオメトリを関連付ける
2. sequence 変更前の display 待ち picture を完了させてから状態を更新する

picture index を利用する場合は、次を管理する。

- 表示幅と表示高さ
- mapped output surface の高さ
- display area
- decoder session の世代
- picture の表示完了後に情報を破棄するライフサイクル

## テスト戦略

モックやスタブは使わず、NVIDIA GPU 実機で検証する。

解像度、segment、表示領域を画素値から識別できるパターン映像を使用する。

単色の黒フレームだけでは、コピー元の位置や Y / UV の混在を検出できないため使用しない。

少なくとも次を検証する。

- 解像度を変更せず、`display_area.left` または `top` が非ゼロの入力
- `display_area.left` または `top` が非ゼロで、decoder 再作成を伴う解像度変更
- coded サイズと display area の幅、高さ、原点のいずれかが異なる入力
- `max_display_delay = 0`
- `max_display_delay > 0` かつ B フレームを含む H.264 または H.265
- 320x240 → 256x160 → 320x240 の縮小と復帰
- 256x160 → 320x240 → 256x160 の拡大と縮小

各出力フレームについて次を確認する。

- フレーム順序とフレーム数
- `width()` / `height()`
- Y / UV の stride とプレーンサイズ
- 既知の座標にある Y / U / V の値
- `display_area` の左上と右下に対応する画素が正しいこと
- sequence 変更境界で、以前の sequence の画素や Y データが UV プレーンへ混入していないこと
- sequence callback と decode / display callback の順序

`cuvidReconfigureDecoder` 経路の検証は issue 0024 で行い、本 issue の完了条件には含めない。

## 完了条件

- `DecodedFrame` の寸法と画素データに関する公開契約が rustdoc に明記されている
- `display_area.left` または `top` が非ゼロの場合も、寸法、stride、Y / UV データが表示領域と一致する
- 通常の decoder 再作成経路にリグレッションがない
- `max_display_delay > 0` と B フレームを含む sequence 変更について、callback の順序と旧 sequence の display 待ち picture の有無が実機で確認されている
- 旧 sequence の display 待ち picture が残る場合は、display callback がその picture と無関係な最新ジオメトリを使用しない
- 旧 sequence の display 待ち picture が残らない場合は、その前提と検証結果がコードコメントまたはテストに記録されている
- パターン映像を使った NVIDIA GPU 実機テストが追加されている
- `CHANGES.md` の `develop` セクションに `[FIX]` エントリが追加されている

## 解決方法

### 変更対象ファイル

- `src/decode.rs` — display area に対応する NV12 コピー処理、遅延フレームの検証に応じたジオメトリ管理、実機テストを修正する
- `testdata/resolution-change/` — segment と Y / UV の位置を識別できるパターン映像、display area の原点が非ゼロの入力、および B フレームを含む入力を追加する
- `README.md` — `DecodedFrame` の寸法と画素データの出力契約を説明する
- `skills/shiguredo-nvcodec/SKILL.md` — decoder の display area と出力ジオメトリの説明を更新する
- `CHANGES.md` — 次の `[FIX]` エントリを追加する

```markdown
- [FIX] Decoder の display area と DecodedFrame の画素領域が一致しない問題を修正する
  - @sile
```

## 関連 issue

- 0006 (closed): decoder 再作成による動的解像度変更を導入した。本 issue では通常の再作成経路をリグレッション対象とし、遅延フレームが残る場合だけ picture ごとの寸法管理を修正する
- 0024 (open): reconfigure と decoder 再作成のハイブリッド化。本 issue で確定した `DecodedFrame` の出力契約を reconfigure 経路へ適用する
