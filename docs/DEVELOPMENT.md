# 開発者向け情報

利用方法・導入手順は[README](../README.md)、実装・検証状況は[STATUS.md](STATUS.md)を参照してください。

## 技術構成

アプリケーション固有のコードはRustです。GUIはegui / eframe、描画はglow / OpenGLを使用します。Electron・Node.js・Pythonランタイム・WebViewは実行に不要です。

主な依存ライブラリはureq、serde / serde_json、Fluent / unic-langid、image、ab_glyph、hound、uuid、libcです。依存関係の条件と原文は[第三者ライセンス一覧](../THIRD_PARTY_LICENSES.md)を参照してください。

翻訳は `locales/<言語>/main.ftl` に分離しています。設定ディレクトリの同じ構成に翻訳を追加すると、次回起動時に言語一覧へ追加されます。不足した項目は英語へフォールバックします。

プロジェクトの検証はソートによる隣接比較と素材IDの索引を使用します。Undoは `src/project/history.rs` の差分履歴で、50件・符号化した差分サイズと固定メタデータの合計64MiBを上限とします（操作中の作業用コピーやプロセス全体のRAM上限を示す値ではありません）。

長いFFmpegフィルターは権限0600の一時ファイルへ保存し、処理後に削除します。外部プロセスは専用のプロセスグループで実行し、キャンセル時はそのグループとパイプ読み取りを停止します。

検索可能な選択UIは `src/ui/searchable_combo_box.rs` にあります。`Choice<T>` に表示名と値を渡し、コントロールごとに `SearchState` と安定したIDを用意します。検索文字列と選択値は別に管理し、候補は表示行だけ描画します。

フォント一覧は既存のFontconfig連携を利用し、導入済みの `fc-list` からワーカーで取得します。フォント名・パス・コレクション番号を保持し、取得失敗時も従来のファイル指定を利用できます。フォント取得・選択によるフォントのコピーや自動導入は行いません。

キャラクター設定はプロジェクト外の `characters.json` へ保存し、クリップはUUID参照と疎な上書きを持ちます。保存競合の保護、schema 4への互換移行、APIと字幕への適用方法は[キャラクター設定の実装](CHARACTERS.md)を参照してください。

## 品質チェック

`cargo-deny` は開発用ツールです。手動導入する場合は次を使用します。

```bash
cargo install --locked --version 0.20.2 cargo-deny
```

```bash
cargo fmt --all -- --check
cargo check --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo deny check licenses
cargo run --locked --bin license-notices
```

通常のテストでは、外部FFmpeg・VOICEVOX・音声デバイスを必要とする試験を除外します。外部試験の実行条件・結果は[開発状況](STATUS.md)を参照してください。

## ライセンス文書

`license-notices` は第三者表示を生成し、原文の欠落時はエラーにします。上流のライセンス文書を自動取得するツールではありません。

- [第三者ライセンス一覧](../THIRD_PARTY_LICENSES.md)
- [ライセンス原文](../LICENSES/)
- [上流文書の取得元・コミット・SHA-256](UPSTREAM_LICENSES.json)
- [依存・ネイティブライブラリ・アセットの監査記録](LICENSE_AUDIT.md)

公開先、GitHubの所有者、正式なLinuxアプリケーションIDは未決定です。名称・商標の利用可否も未確認です。
