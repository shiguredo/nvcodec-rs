# 0024-change-decoder-resolution-change-strategy

- Created: 2026-08-05
- Branch: feature/change-decoder-reconfigure-strategy
- Updated: 2026-08-19

## 目的

Sora の WebRTC シミュキャスト録画のように、1 つの MP4 内でシーケンスヘッダ (SPS / PPS / VPS 等) と符号化解像度が変化するストリームで、シーケンス変更ごとに `cuvidCreateDecoder` を呼び直すコストを削減する。

利用側にストリームの最大解像度を指定させず、crate が `cuvidReconfigureDecoder` と decoder の再作成を使い分けられるようにする。reconfigure を常に試みるか、利用側が従来の再作成方式を選べるようにするかは、実装と実機検証でリスクを確認してから決定する。

## 現状

`develop` の `src/decode.rs` は、`pfnSequenceCallback` で解像度変更を検出すると、既存 decoder を `cuvidDestroyDecoder` で破棄してから `cuvidCreateDecoder` で再作成する。

この方式は最大解像度を事前に知る必要がなく確実だが、シーケンス変更ごとに decoder の再作成コストが発生する。`CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` も、そのシーケンスの `format.coded_width` / `coded_height` と同じ値になるため、`cuvidReconfigureDecoder` 用の上限として活用されていない。

当初は `DecoderConfig` に `max_coded_width` / `max_coded_height` を追加し、利用側が最大 coded サイズを指定した場合だけ reconfigure する方針だった。しかし、利用側が sequence header とは別に最大 coded サイズを把握する必要があり、上限の見積もりを誤ると、それまでデコードできていた解像度変化が終端エラーになる。

decoder は既に sequence callback で coded サイズを取得でき、上限を超えた場合に使える再作成経路も持っている。SDK 固有の reconfigure 制約は crate 内部で吸収できるため、最大 coded サイズを公開設定にする必要はない。

## 設計方針

### 最大 coded サイズを公開 API にしない

`DecoderConfig` に `max_coded_width` / `max_coded_height` は追加しない。SDK が要求する decoder session ごとの上限は crate 内部で管理する。

### reconfigure の利用を選べる公開設定を追加する

`DecoderConfig` に `reconfigure_enabled: bool` を追加し、利用側が「reconfigure を可能なら使用する」と「常に decoder を再作成する従来方式」を選択できるようにする。

- `false` (推奨値) は従来方式で、シーケンス変更ごとに decoder を破棄して再作成する
- `true` は、現在の decoder session の上限以内の解像度変化を `cuvidReconfigureDecoder` で処理し、上限を超える拡大やコーデック情報の変化は再作成する
- 後方互換のため、デフォルト相当は従来方式 (`false`) とする。公開フィールドの追加は既存の struct literal を壊すため後方互換のない変更として扱う

公開設定は reconfigure の利用方針だけを表し、session 上限の管理と上限超過時の再作成は crate が行う。利用側に最大 coded サイズは要求しない。

`reconfigure_enabled == true` は `max_display_delay > 0` と組み合わせられない。reconfigure は decoder を残すため、表示遅延中の旧 sequence の picture が新しいジオメトリでコピーされる可能性がある。この組み合わせは `Decoder::new` が設定エラーとして拒否する。`reconfigure_enabled == false` (従来方式) では delay の制約はない。

### decoder session ごとの coded サイズ上限

`DecoderState` に、現在の decoder session を作成したときの `ulMaxWidth` / `ulMaxHeight` を保持する。

- 初回 sequence callback では、`format.coded_width` / `coded_height` を `CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` に設定する
- reconfigure では、現在の coded サイズが保存した上限以下かを判定する
- decoder を再作成した場合は、再作成に使用した `format.coded_width` / `coded_height` を新しい session の上限として保存する
- 上限と作成時ジオメトリは、`cuvidCreateDecoder` が成功した後にだけ更新する

再作成時の上限は、そのときの coded width / height の組をそのまま使用する。過去の最大幅と最大高さを別々に合成すると、実際には入力されていない大きな矩形を上限にして GPU メモリ消費や hardware の最大マクロブロック数制約を増やす可能性があるため、採用しない。

### `cuvidReconfigureDecoder` の適用条件

以下をすべて満たす場合だけ `cuvidReconfigureDecoder` を呼ぶ。

- decoder が作成済みである
- `format.coded_width` / `coded_height` が現在の decoder session の上限以下である
- 直前の create / reconfigure 時から `codec` / `chroma_format` / `bit_depth_luma_minus8` / `bit_depth_chroma_minus8` / `progressive_sequence` が変化していない

判定用ベースラインは `DecoderState` に保存する。decoder を再作成した場合は、新しい `CUVIDEOFORMAT` からベースラインを必ず更新し、次回以降 reconfigure 経路へ戻れるようにする。

### sequence callback の分岐

reconfigure を利用する経路では、`src/decode.rs` の `DecoderState::handle_video_sequence` が以下の順で処理する。

1. `format.display_area` と coded サイズを検証する
2. decoder が未作成なら、現在の coded サイズを上限として作成する
3. codec 情報、chroma format、bit depth、progressive sequence のいずれかが変化していたら、現在の coded サイズを上限として再作成する
4. 現在の coded サイズが decoder session の上限を超えていたら、現在の coded サイズを上限として再作成する
5. それ以外は `cuvidReconfigureDecoder` で in-place に再構成する
6. 成功した経路に合わせて、coded サイズ上限、判定用ベースライン、作成時ジオメトリ、現在のフレーム寸法を更新する
7. `format.min_num_decode_surfaces` を parser へ返す

縮小後に元のサイズへ戻る場合は、元のサイズが同じ decoder session の上限以内なので再作成しない。一方、小さい解像度から開始して上限を超える解像度へ拡大した場合は一度再作成し、その後は新しい session の上限以内で reconfigure する。

常に decoder を再作成する方式を公開する場合、その経路では上記の適用判定と reconfigure を行わず、初回以外の sequence callback ごとに既存の destroy + create 経路を使用する。

### reconfigure 失敗時のフォールバック

公開設定なしで reconfigure を常に試みる方式を採用するには、reconfigure を使用したことによって、従来の再作成方式なら継続できた入力が終端エラーにならないことが必要である。

`cuvidReconfigureDecoder` が失敗した場合は、`DecoderStats::total_reconfigure_failure_count` をインクリメントする。失敗後の decoder を安全に破棄できることが確認できた場合は、現在の `CUVIDEOFORMAT` から decoder を再作成する。再作成に成功した場合はデコードを継続し、`total_create_decoder_count` にも反映する。

NVDEC SDK は reconfigure 失敗後の decoder 状態を明示していないため、失敗した decoder を継続利用しない。安全な再作成フォールバックが成立する場合は、`cuvidDestroyDecoder` または後続の `cuvidCreateDecoder` まで失敗したときだけ、既存の終端契約に従って原因エラーを通知し、当該 `Decoder` を終端状態にする。

このフォールバックが実機で成立することを確認する。reconfigure 失敗後の `cuvidDestroyDecoder` を安全に実行できない、または再作成方式にはなかった終端エラーを十分に避けられない場合は、常時 reconfigure を採用せず、利用側が従来方式を選択できる公開設定を追加する。その場合、reconfigure を選択した経路では reconfigure 失敗時に終端することを公開契約として明記する。

### decoder 再作成失敗時の状態

decoder 再作成は既存の destroy + create 経路を利用する。`cuvidCreateDecoder` が失敗すると decoder は `null` のままになり、既存の終端契約によって当該 `Decoder` は使用不能になる。

`format.display_area` など SDK 呼び出し前に検証できる値は、既存 decoder を破棄する前に検証する。検証エラーで使用可能な decoder を破棄しない。

### `CudaLibrary` への追加

`src/lib.rs` の `CudaLibrary::load` で `cuvidReconfigureDecoder` の存在を確認し、`cuvid_create_decoder` / `cuvid_destroy_decoder` と同じ形式の `cuvid_reconfigure_decoder` ラッパーを追加する。

### 出力ジオメトリ契約との依存関係

issue 0031 は、develop ブランチの display area の不整合を修正し、`DecodedFrame` の寸法、stride、Y / UV データに関する出力契約を確定した (PR #22)。

issue 0031 は reconfigure 実装に依存せず、develop ブランチの decoder 再作成経路だけで完了した。

本 issue は、issue 0031 で確定した出力契約を reconfigure 経路へ適用する責務を持つ。

現在の reconfigure 実装案には、`CUVIDRECONFIGUREDECODERINFO.ulTargetWidth` / `ulTargetHeight` と `display_area` を decoder 作成時の値に固定する一方で、`DecodedFrame` の寸法と NV12 コピー位置を新しい coded サイズと display area から計算する不整合がある。

たとえば 320x240 の target surface を維持したまま 256x160 へ reconfigure すると、mapped output surface の UV プレーンは target 高さ 240 の後ろから始まる。一方、新しい coded 高さ 160 から UV オフセットを計算すると、Y プレーンの途中を UV データとしてコピーする可能性がある。

この不整合は未マージの reconfigure 実装に固有であり、develop ブランチの既存不具合としては扱わない。ただし、本 issue を develop ブランチへマージする前に解消する。

reconfigure 経路では、decoder session の作成時ジオメトリと、現在の sequence の coded サイズおよび display area を別々に管理する。

少なくとも次を実機で比較する。

1. 新しい coded サイズと display area に合わせて target サイズも更新する
2. NVIDIA 公式サンプルと同様に、作成時の target サイズと display area を維持する

作成時の target サイズを維持する場合は、UV プレーン開始位置を実際の mapped output surface の高さから計算し、その surface から現在の display area に対応する Y / UV データを取り出す。

新しい target サイズへ更新する場合も、`cuvidReconfigureDecoder`、後続の `cuvidDecodePicture`、`cuvidMapVideoFrame` が対象 codec と GPU / driver で成功することを確認する。

どちらの方式でも、利用側から見える解像度を作成時サイズへ暗黙に固定してはならない。

reconfigure で issue 0031 の出力契約を維持できない条件は、decoder 再作成へフォールバックする。

黒一色のテストデータではコピー元の不一致を検出できないため、issue 0031 で用意したパターン映像を使用し、既知の座標にある Y / U / V の値まで検証する。

### reconfigure の実装・検証項目

公開設定 (`reconfigure_enabled`) を追加し、デフォルトは従来方式 (`false`) とすることで、利用側の選択肢を公開する方針に決定した。reconfigure 経路の実装・調査で次の観点を確認する。

- reconfigure 失敗後に decoder を安全に破棄し、現在の sequence から再作成して継続できるか
- 解像度変更前後で `DecodedFrame` の寸法、stride、Y / UV のコピー位置と内容が正しいか
- `max_display_delay > 0` との組み合わせは `Decoder::new` で拒否する (reconfigure は decoder を残すため、表示遅延中の旧 sequence の picture が新ジオメトリでコピーされる可能性がある)
- 縮小後も大きい session 上限を保持することで、同時 decoder 数や GPU メモリ消費へ許容できない影響が出ないか
- 対象 codec と CI / 利用環境の GPU・driver の組み合わせで、reconfigure 固有の失敗や出力差が発生しないか
- destroy + create と比較して、reconfigure に採用する価値がある処理時間・latency の改善を確認できるか

調査結果と最終判断は、本 issue の「解決方法」に根拠とともに記録する。画素一致 (Y / UV データ) の実機検証は、パターン映像テストデータの整備が必要であり、issue 0031 でも非ゼロ原点の画素一致は未達のため、本 issue では対応せず残課題として記録する。

## 完了条件

- `max_coded_width` / `max_coded_height` が公開 API に追加されていない
- `DecoderConfig` に `reconfigure_enabled: bool` が追加され、デフォルト相当が `false` (従来方式) である
- `reconfigure_enabled == true` かつ `max_display_delay > 0` の組み合わせが `Decoder::new` で設定エラーとして拒否される
- 初回 sequence callback で、最初の coded サイズが `CUVIDDECODECREATEINFO.ulMaxWidth` / `ulMaxHeight` と内部の session 上限に設定される
- `reconfigure_enabled == true` で 320x240 → 256x160 → 320x240 のストリームをデコードしたとき、`total_create_decoder_count` が 1、`total_reconfigure_decoder_count` が 2 以上になり、全フレームが欠落なくデコードされる
- `reconfigure_enabled == true` で 256x160 → 320x240 → 256x160 のストリームをデコードしたとき、320x240 への変更時に decoder が再作成され、その後の 256x160 への変更では reconfigure される
- `reconfigure_enabled == false` で解像度変化ストリームをデコードしたとき、従来どおりシーケンス変更ごとに decoder が再作成され、全フレームが欠落なくデコードされる
- codec / chroma format / bit depth / progressive sequence のいずれかが変化した場合は decoder が再作成され、成功後は reconfigure 経路へ戻れる
- `cuvidReconfigureDecoder` 失敗後の decoder を安全に破棄して再作成できるかが実機で確認され、結果に応じた失敗時契約が確定している
- 安全な再作成フォールバックを採用する場合は、reconfigure と再作成が両方失敗した場合だけ原因エラーが通知され、当該 `Decoder` が終端する
- 安全な再作成フォールバックを採用できない場合は、常時 reconfigure を採用せず、reconfigure を選択した経路の終端条件が公開 API と文書に明記されている
- `format.display_area` などの事前検証に失敗した場合は、既存 decoder が破棄されない
- `DecoderStats::total_create_decoder_count` / `total_reconfigure_decoder_count` / `total_reconfigure_failure_count` が各経路を正しく反映する
- reconfigure 経路で、作成時 target surface と現在の sequence のジオメトリが区別して管理されている
- reconfigure で出力契約を維持できない条件が、decoder 再作成へフォールバックする条件として明文化されている
- 画素一致 (Y / UV データ) の実機検証は、パターン映像テストデータの整備が別途必要なため残課題とし、本 issue の完了条件に含めない
- `README.md` と `skills/shiguredo-nvcodec/SKILL.md` が、利用側の最大解像度指定を要求せず、最終決定した reconfigure 方針を説明している
- `CHANGES.md` の `develop` セクションに、最終的な公開 API と挙動に対応するエントリが追加されている

## 解決方法

実装と実機調査の完了後に、公開設定の判断チェックポイントの結果、採用する方式、公開 API の最終形を本節へ記録する。

### 変更対象ファイル

- `src/decode.rs` — `DecoderConfig.reconfigure_enabled` の追加と検証、session ごとの coded サイズ上限、reconfigure 適用判定、作成時 target surface と現在の sequence のジオメトリ管理、出力契約を維持する NV12 コピー処理、再作成フォールバック、統計値更新、実機テストを追加する。`DecoderConfig` の `max_coded_width` / `max_coded_height` とその検証は追加しない
- `src/lib.rs` — `cuvidReconfigureDecoder` の存在確認とラッパーを追加する
- `build.rs` — docs.rs 用スタブに `CUVIDRECONFIGUREDECODERINFO` を追加する
- `testdata/resolution-change/` — 解像度変化テストデータを使用し、縮小と上限超過後の縮小を検証する
- `README.md` — 最大 coded サイズを指定せずに解像度変化へ追従することと、最終決定した reconfigure 方針を説明する
- `skills/shiguredo-nvcodec/SKILL.md` — 動的解像度変更の説明を最終決定した reconfigure 方針へ更新する
- `CHANGES.md` — `develop` セクションに最終的な公開 API と挙動に対応するエントリを追加する

`CHANGES.md` のエントリは次の内容とする。`reconfigure_enabled` の追加は既存の struct literal を壊すため、後方互換のない `[CHANGE]` として記載する。

```markdown
- [CHANGE] `DecoderConfig` に `reconfigure_enabled` フィールドを追加する
  - `false` (推奨値) は従来どおりシーケンス変更ごとに decoder を再作成する
  - `true` は現在の decoder session の上限以内の解像度変化を `cuvidReconfigureDecoder` で再構成する
  - `true` は `max_display_delay > 0` と組み合わせられない (組み合わせた場合は `Decoder::new` が設定エラーを返す)
  - @sile
```

開発ブランチ内の中間設計である `max_coded_width` / `max_coded_height` の追加と削除は変更履歴に記載しない。

## 実装で判明した事項

- `query_decoder_caps` で得られる codec ごとの hardware 最小デコード解像度を下回るテストデータでは、sequence callback、decoder 作成、reconfigure が成功しても、`cuvidDecodePicture` が `CUDA_ERROR_INVALID_VALUE` を返す。テストデータは全対象 codec の最小値を上回る 256x160 以上を使用する
- parser の DPB 数と decoder の decode surface 数の同期は 0028 で修正済み。本 issue の create / reconfigure 経路は、develop に実装済みの実効 surface 数決定処理 (`DecoderState::determine_num_decode_surfaces`) を使用する
- max coded サイズ超過を終端エラーにしなくなるため、0029 から後回しになっていた終端契約テストの安定したエラー誘発手段としては使用できない。終端契約テストは本 issue のスコープに含めない

## 関連 issue

- 0006 (closed): 解像度変更ごとに decoder を再作成する現在の方式を導入した。本 issue は再作成経路をフォールバックとして残す
- 0017 (pending): destroy-then-create 順序による復旧不能問題。「display_area 検証位置」は本 issue で解消するが、再作成経路自体は残るため、destroy-then-create 順序の問題は残る
- 0027 (closed): `DecoderStats` を追加した。本 issue は既存の create / reconfigure / failure カウンターで各経路を検証する
- 0028 (closed): parser の DPB 数と decoder の decode surface 数を同期した。本 issue の create / reconfigure 経路は、develop に実装済みの実効 surface 数決定処理 (`DecoderState::determine_num_decode_surfaces`) を使用する
- 0029 (closed): デコードエラー後の `Decoder` を終端状態にした。本 issue の再作成フォールバックまで失敗した場合は、この終端契約に従う
- 0031 (closed): develop ブランチの display area の不整合を修正し、`DecodedFrame` の出力契約を確定した。本 issue は、その契約を reconfigure 経路へ適用する
- 0033 (closed): 遅延フレームを伴う sequence 変更で、destroy + create 経路では利用者に見える誤適用が観測されなかった。decoder を残す reconfigure 経路での待ち picture 混在は、本 issue の公開設定判断チェックポイントで再検証する
