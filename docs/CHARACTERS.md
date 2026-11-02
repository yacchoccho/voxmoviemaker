# キャラクター設定とクリップ継承

## データ構造

`CharacterLibrary`（schema_version 1）はUUIDで識別する `Character` の一覧です。同名・同じVOICEVOX話者のキャラクターも独立して保存します。

`Character` はID、名前、エンジン種別、`VoicevoxSettings`、字幕用 `TextSettings` を持ちます。現在のエンジン種別はVOICEVOXのみです。キャラクターの字幕には発話本文を保存せず、本文はクリップに保持します。

話者UUIDは `speaker_uuid`、APIに渡すスタイルIDは互換性を保つため従来の `speaker` フィールドを使います。表示名は識別子に使いません。基本設定で話者を変更すると、その話者のスタイルだけを表示し、有効な初期スタイルへ切り替えます。

クリップの `character` は省略可能な `CharacterBinding` で、キャラクターID、`VoiceOverrides`、`SubtitleOverrides`、字幕の有効状態を保存します。内部のBoxはメモリ上の配置だけに影響し、JSON形式には影響しません。

```json
{
  "character_id": "12345678-1234-4234-8234-123456789abc",
  "voice": { "speed": 1.5, "stereo": false },
  "subtitle": { "x": 500.0, "size": 72.0 },
  "subtitle_enabled": true
}
```

上書きしていない項目はJSONから省略します。0・false・空のフォントパスも明示的な値として保持します。句読点設定の `{"value": null}` はENGINEの既定値を使う明示指定で、継承のための項目省略とは区別します。

## 設定の解決と音声生成

優先順位は個別上書き、キャラクター共通設定、VMM既定値です。`CharacterBinding` の解決処理を、生成・再生成、プロパティ表示、共通RenderPlanで使用します。個別上書きはキャラクターの割り当て変更でも保持します。

新規生成は入力欄で選択したキャラクターIDから設定を取得します。入力欄に話者の直接選択はありません。未選択・欠損キャラクター・未接続・無効な話者とスタイルの場合は生成を開始せず、本文を保持します。クリップの選択と新規生成用キャラクターの選択は別に扱います。

再生成は選択クリップのキャラクターIDと個別上書きを使い、最新の共通設定を解決します。既存WAVは置換せず、新しいUUIDのWAVを作成してからクリップを更新します。API失敗・衝突・処理中のクリップ変更では既存クリップとWAVを保持します。

共通字幕の変更は、プロジェクトを書き換えずに次の描画へ反映します。音声設定の変更では自動生成せず、WAVへの反映には明示的な再生成が必要です。

## 保存と互換性

共通設定の保存先は `$XDG_CONFIG_HOME/voxmoviemaker/characters.json`、未設定・空・相対パスの場合は `~/.config/voxmoviemaker/characters.json` です。起動時とプロジェクト切替時に読み込み、管理画面の「再読み込み」でも更新できます。

読み書きは `CharacterStore` に集約しています。作成・編集・複製・削除時は、専用の `characters.lock` のflock、読み込み時点の内容との一致確認、権限0600の一時ファイルへの書き込み、atomic renameの順で保存します。ロックは子プロセスへ一時的に継承されたFDがあっても明示的に解放します。

別インスタンスによる更新があれば保存を拒否し、再読み込みを案内します。保存失敗時は永続データを更新せず、管理画面の編集内容を保持して再試行できます。破損・未対応スキーマ・読み込み不能のファイルは保護し、修復後の再読み込みまで保存しません。

キャラクター名はIME変換中や一時的な空欄では保存せず、確定した有効な名前だけを保存します。入力欄にはキャラクターIDに基づく固定IDを使い、変換確定のEnterやエラー表示、全消去でもフォーカスを維持します。

プロジェクトはschema_version 4です。schema 1〜3を読み込み時に移行し、キャラクターIDのないクリップは従来の音声・字幕設定を使用します。読み込みだけでは共通キャラクターを作成・変更しません。

新規キャラクター音声のプロジェクトにはIDと上書きだけを保存し、共通設定を複製しません。共通設定はアセット付き書き出しにも同梱しません。別環境へ移す場合は `characters.json` を別途管理するか、キャラクターを再割り当てしてください。

欠損キャラクターはIDを保持し、不明なキャラクターとして表示します。生成済み音声は引き続き利用でき、字幕はVMM既定値と個別上書きで描画します。同じIDが再び利用可能になると共通設定を解決できます。

## 字幕とフォント

`TextSettings.position` はプロジェクトのピクセル座標で、動画左上が原点です。中央揃えのXは文字列の中心、左揃えは左端、右揃えは右端です。Yは既存の上・中央・下の基準点に対応し、下基準なら文字ブロックの下端です。

位置・文字サイズは描画解像度に合わせて変換し、低解像度プレビューで保存値は変更しません。旧クリップの `position: null` は以前の自動配置を保持します。文字色・縁取り・太さ・不透明度・配置を既存のRust描画処理で扱い、プレビュー・プロキシ・書き出しは共通RenderPlanを使います。

フォントは既存の検索可能コンボボックスと非同期Fontconfig一覧を再利用します。ファイルパスとコレクション番号を保持し、フォントのコピーや同梱は行いません。

## VOICEVOX APIとの対応

[公式API](https://voicevox.github.io/voicevox_engine/api/)、[AudioQueryの定義](https://github.com/VOICEVOX/voicevox_engine/blob/master/voicevox_engine/model.py)、[synthesisの定義](https://github.com/VOICEVOX/voicevox_engine/blob/master/voicevox_engine/app/routers/tts_pipeline.py)を確認しています。APIの許容範囲内で、VMMは次の入力範囲を設けています。

| 設定 | VMMの範囲・送信先 |
| --- | --- |
| 話速 | 0.5〜2.0 / AudioQuery.speedScale |
| 音高 | -0.15〜0.15 / AudioQuery.pitchScale |
| 抑揚・合成音量 | 0〜2.0 / intonationScale・volumeScale |
| 前後の無音 | 0〜1.5秒 / prePhonemeLength・postPhonemeLength |
| 句読点の無音 | ENGINE既定値または0〜10秒 / pauseLength |
| 句読点の無音倍率 | ENGINE既定値または0〜5 / pauseLengthScale |
| 出力レート | 8000〜192000 Hz / outputSamplingRate |
| ステレオ | outputStereo |
| 疑問文調整 | synthesisのenable_interrogative_upspeakクエリ引数 |

句読点設定は取得したAudioQueryに該当キーがある場合だけ変更します。明示指定したキーがなければ、合成要求を送る前にエラーにします。未指定ならENGINEから返された値を維持します。

疑問文調整の有効時は公式APIの既定値を使用します。無効を要求した場合はENGINEのOpenAPIで対応するクエリ引数を確認し、未対応ならエラーにします。`accent_phrases` など発話ごとの情報はキャラクター共通設定へ保存しません。

## 検証と制約

通常テストは継承・明示値・保存競合・破損保護・旧形式・欠損参照・座標変換・GUI管理操作・キャラクター別の合成要求を検査します。実ENGINE用の `tests/voicevox_live.rs` は明示実行が必要で、ユーザーが起動済みのENGINEと導入済みFFmpegだけを使用します。

2026-11-02にVOICEVOX ENGINE 0.26.0で生成・再生成・24000 Hzステレオ・追加設定・WAV保持・保存再読み込み・字幕付きMP4を確認しました。実IME・OSダイアログ・実スピーカーでの手動評価は今回の検証に含みません。別インスタンスの変更との自動マージ、キャラクター設定自体のUndo、キャラクター設定の同梱エクスポートはありません。

## 変更ファイル

- モデル・永続化：`src/characters/mod.rs`、`src/characters/storage.rs`、`src/project/model.rs`、`src/project/persistence.rs`。
- アプリ処理：`src/app/mod.rs`、`src/app/import.rs`、`src/app/text.rs`、`src/app/transport.rs`、`src/settings.rs`、`src/lib.rs`。
- UI：`src/ui/characters.rs`、`src/ui/voice_input.rs`、`src/ui/properties.rs`、`src/ui/timeline.rs`、`src/ui/dialogs.rs`、`src/ui/mod.rs`。
- API・描画：`src/voicevox/mod.rs`、`src/media/render_plan.rs`、`src/media/text.rs`。
- 翻訳：`locales/ja-JP/main.ftl`、`locales/en-US/main.ftl`、`locales/zh-CN/main.ftl`。
- テスト：`tests/characters.rs`、`tests/characters_api.rs`、`tests/gui.rs`、`tests/voicevox_live.rs`、および既存クリップ型に追従する `core.rs`、`phase5.rs`、`playback.rs`、`processes.rs`、`review_fixes.rs`、`ffmpeg_integration.rs`。サンプル `examples/playback-fixture.rs` も型に追従します。
- 文書：`README.md`、`docs/DEVELOPMENT.md`、`docs/STATUS.md`、この文書。Cargoの依存関係と許可ライセンスの追加・変更はありません。
