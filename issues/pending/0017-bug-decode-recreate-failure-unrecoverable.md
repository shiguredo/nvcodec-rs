# 0017-bug-decode-recreate-failure-unrecoverable

Created: 2026-05-10
Model: deepseek-v4-pro

## Pending 理由

コードが複雑になる割に効果が限定的な上に完璧な実装が不可能であるため

## 背景

issue 0006 で `handle_video_sequence_inner` にストリーム中の解像度変更対応が実装された。この実装では、古いデコーダーを `cuvid_destroy_decoder` で破棄してから新しいデコーダーを `cuvid_create_decoder` で作成する（destroy-then-create 順序）。この順序のため、新規作成に失敗すると古いデコーダーが既に存在しない状態から回復できない。

## 問題箇所

`src/decode.rs` `handle_video_sequence_inner` 関数内。2 つの問題が存在する:

### 問題 1: destroy-then-create の順序

古いデコーダーを破棄（L434-439）:
```rust
if !state.decoder.is_null() {
    state
        .lib
        .with_context(state.ctx, || state.lib.cuvid_destroy_decoder(state.decoder))?;
    state.decoder = ptr::null_mut();
}
```

その後、新しいデコーダーを作成（L465-469）:
```rust
state.lib.with_context(state.ctx, || {
    state
        .lib
        .cuvid_create_decoder(&mut state.decoder, &mut create_info)
})?;
```

### 問題 2: `display_area` 検証が `cuvid_create_decoder` の後にある

L470-486 の `display_area` バリデーションが L465-469 の `cuvid_create_decoder` より後に実行される。作成済みの新デコーダーと破棄済みの古デコーダーがある中途半端状態で Err が返る。

## 問題

`cuvid_create_decoder` が失敗すると:

1. `state.decoder` は null のまま
2. パーサーは生きているため、後続フレームの FFI コールバックは引き続き発生する（戻り値 0 でパーサーが停止するかどうかは NVDEC SDK の挙動に依存するが、いずれにせよ古いデコーダーは破棄済みで復旧不能である）
3. `handle_picture_decode_inner` と `handle_picture_display_inner` は null チェックでエラーを返す
4. `run_worker` ループ内で `state.decode()` はパーサー生存のため Ok を返し続け、毎ジョブ `drain_frames` → `callback(Err(...))` が無限に繰り返される

## 再現手順

1. ストリームの解像度変更を伴う映像データをデコードする
2. 解像度変更時に GPU メモリ不足などで `cuvid_create_decoder` が失敗する状況を作る（例: 複数デコーダーを同時実行し GPU メモリを枯渇させる）
3. 後続フレームがすべてデコード失敗し、エラーコールバックが繰り返し呼ばれる

## 推奨対応

### 基本方針

create-then-destroy 順序に変更する。**create が成功した場合のみ古いデコーダーを破棄する。**

create が失敗した場合、古いデコーダーは新しい解像度のデータを正しくデコードできないため、古いデコーダーも破棄し `state.decoder` を null にする。失敗パスの最終状態は現行コードと同じ（decoder null + parser 生存）だが、destroy-then-create と異なり成功パスでは古いデコーダーの破棄を新しいデコーダーの作成成功後に遅延できる。

### 修正後の疑似コード

```rust
// 既存デコーダーを破棄し、state.decoder を null にするヘルパー
fn destroy_old_decoder_and_null(state: &mut DecoderState) {
    if !state.decoder.is_null() {
        let _ = state.lib.with_context(state.ctx, || {
            state.lib.cuvid_destroy_decoder(state.decoder)
        });
        state.decoder = ptr::null_mut();
    }
}

fn handle_video_sequence_inner(state, format) -> Result<i32> {
    // Step 1: display_area の検証を最初に行う
    //         coded_width/coded_height の 0 チェックも含める
    if format.coded_width == 0 || format.coded_height == 0 {
        destroy_old_decoder_and_null(state);
        return Err(Error("coded_width or coded_height is zero"));
    }
    let left = format.display_area.left;
    let right = format.display_area.right;
    let top = format.display_area.top;
    let bottom = format.display_area.bottom;
    if left < 0 || top < 0 || right <= left || bottom <= top
        || right as u32 > format.coded_width
        || bottom as u32 > format.coded_height
    {
        destroy_old_decoder_and_null(state);
        return Err(Error("invalid display_area"));
    }

    // Step 2: create_info を構築（現行 L441-463 を維持）
    //         vidLock = state.ctx_lock も含む
    //         ctx_lock は新旧デコーダー間で共有されるが、
    //         NVDEC SDK はパーサーとデコーダー間の lock 共有を許容しており問題ない
    let mut create_info = build_create_info(state, format);

    // Step 3: 新しいデコーダーを作成（一時変数）
    let mut new_decoder = ptr::null_mut();
    if let Err(e) = state.lib.with_context(state.ctx, || {
        state.lib.cuvid_create_decoder(&mut new_decoder, &mut create_info)
    }) {
        // create 失敗時: 古いデコーダーも破棄する
        // （新しい解像度のデータを古いデコーダーでデコードできないため）
        destroy_old_decoder_and_null(state);
        return Err(e);
    }

    // Step 4: create 成功 → 古いデコーダーを破棄
    if !state.decoder.is_null() {
        if let Err(e) = state.lib.with_context(state.ctx, || {
            state.lib.cuvid_destroy_decoder(state.decoder)
        }) {
            // 古いデコーダーの破棄に失敗した場合:
            // - 古いデコーダーの内部状態は不明だが、破棄 API が失敗した以上
            //   安全に使用し続けることはできない
            // - 新デコーダーを破棄してリークを防ぐ
            let _ = state.lib.with_context(state.ctx, || {
                state.lib.cuvid_destroy_decoder(new_decoder)
            });
            state.decoder = ptr::null_mut();
            return Err(e);
        }
    }
    state.decoder = new_decoder;

    // Step 5: 解像度情報を更新
    state.width = (right - left) as u32;
    state.height = (bottom - top) as u32;
    state.surface_width = format.coded_width;
    state.surface_height = format.coded_height;

    Ok(format.min_num_decode_surfaces as i32)
}
```

全パスの状態遷移:

| パス | 新デコーダー | 古デコーダー | `state.decoder` | `state.width` 等 | 戻り値 |
|------|-------------|-------------|-----------------|-----------------|--------|
| display_area 検証失敗 | 未作成 | 破棄済み | null | 未更新 | Err |
| create 失敗 | null | 破棄済み | null | 未更新 | Err |
| create 成功 + destroy 成功 | 有効 | 破棄済み | 新デコーダー | 更新済み | Ok |
| create 成功 + destroy 失敗 | 有効→破棄 | 維持 (状態不明) | null | 未更新 | Err |

注意点:
- destroy 失敗時、古いデコーダーは NVDEC 内部で部分的に破壊されている可能性があるため、`state.decoder` を null クリアする。新旧両方の GPU メモリリークの可能性があるが、アプリケーション全体の回復不能に比べれば許容する。失敗した destroy を再度試行すると二重破棄の危険があるため行わない
- 全エラーパスで `state.decoder` は null になり、後続の `handle_picture_decode_inner` / `handle_picture_display_inner` は null チェックでエラーを返すため、不正な解像度値が使われることはない

### エラー通知

- `handle_video_sequence_inner` が Err を返した場合、既存の `frame_tx.send(Err(e))` でエラーを利用者に通知する
- 利用者側はコールバックでエラーを受け取り、`Decoder<T>` を drop する。drop 時に `Drop` 実装がパーサーを安全に破棄する
- 後続の `handle_picture_decode_inner` / `handle_picture_display_inner` は `state.decoder` が null のためエラーを返す（既存の動作と同一）

### パーサー停止について

FFI コールバック内で `cuvidDestroyVideoParser` を呼ぶことは、コールバック復帰後にパーサーが解放済みメモリを参照する危険があるため行わない。パーサーは `DecoderState::Drop` で破棄される。

## 変更対象ファイル

- `src/decode.rs` — `handle_video_sequence_inner` の create 順序変更と `display_area` 検証位置の移動
- `CHANGES.md` — `[FIX]` エントリを `## develop` に追加

## CHANGES.md 追記予定

```markdown
- [FIX] 解像度変更時のデコーダー再作成が失敗した場合に状態が回復不能になる問題を修正する
  - 新しいデコーダーを作成してから古いデコーダーを破棄する順序に変更する
  - @担当者
```

## 後方互換

公開 API に変更なし。修正後、解像度変更の成功パスでは振る舞いは変わらない（古いデコーダー破棄 → 新しいデコーダー作成の実質的結果は同じ）。失敗パスでは従来と同じく `state.decoder` が null になりエラーが通知される。

## テスト戦略

GPU 依存のため `cuvid_create_decoder` 失敗を意図的に再現する mock 機構は現状存在しない。以下の方法で修正の正しさを確認する:

- **既存テストの再実行**: `src/decode.rs` 内の `#[cfg(test)] mod tests` を実行し、解像度固定のデコードが引き続き成功することを確認する
- **手動テスト**: 解像度変更を含むストリームを実機でデコードし、変更前後で正しいフレームが出力されることを確認する
- PBT / Fuzzing は不要（GPU 依存のエラーパスであり、新たなロジック追加が限定的なため）

修正の妥当性は、疑似コードとパステーブルを照合したコードレビューによって全分岐の網羅性を確認する。
