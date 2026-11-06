# ライセンス監査記録

対象: VoxMovieMaker 0.1.0 開発版、Linux x86_64-unknown-linux-gnu。確認日: 2026-10-25（日本時間）。実行した検査結果と未確認範囲を分けて記録します。

## 方針と個別確認

独自コードはMITです。権利者として仮の会社・GitHub所有者は設定せず、LICENSEはVoxMovieMaker contributorsの表記にしています。公開先/アプリケーションIDおよび名称・商標の確認は未完了です。

`deny.toml`の許可ライセンスはMIT、MIT-0、Apache-2.0、BSD-2-Clause、BSD-3-Clause、ISC、Zlib、CC0-1.0です。デュアルライセンスは許可される選択肢を利用し、MIT OR Apache-2.0ではMITを選びます。AND条件は全項目の充足が必要です。ライブラリのライセンスをMITへ変更しません。

Unicode-3.0は、初回のcargo-deny失敗を受けて個別に確認しました。経路は主に`eframe → egui-winit → webbrowser → url → idna → ICU4X`、`fluent-bundle → unic-langid → tinystr`、およびRust/Unicode識別子の処理です。実際のLICENSEを読み、著作権/許諾文の保持と権利者名の宣伝利用制限を確認しました。2026-10-24にユーザーが追加を明示承認したため、許可リストへ追加しています。原文も収録します。GPL/LGPL/MPL等のクレートを組み込む例外は追加していません。

設定仕様は[cargo-deny公式](https://embarkstudios.github.io/cargo-deny/checks/licenses/cfg.html)で確認し、0.20.2を使用しました。`include-dev = true`、`include-build = true`として開発・ビルド依存を含めます。Linux GNU以外のターゲットや追加featureは、この監査の配布承認範囲外です。

## 直接依存

| クレート | ロックされた版 | 宣言ライセンス / 採用方針 |
|---|---|---|
| eframe | 0.33.2 | MIT OR Apache-2.0、MITを選択 |
| egui（eframe経由） | 0.33.3 | MIT OR Apache-2.0、MITを選択 |
| serde | 1.0.229 | MIT OR Apache-2.0、MITを選択 |
| serde_json | 1.0.154 | MIT OR Apache-2.0、MITを選択 |
| ureq | 3.5.0 | MIT OR Apache-2.0、MITを選択、TLS/default featuresなし |
| fluent-bundle | 0.16.0 | MIT OR Apache-2.0、MITを選択 |
| unic-langid | 0.9.6 | MIT OR Apache-2.0、MITを選択 |
| uuid | 1.26.1 | Apache-2.0 OR MIT、MITを選択 |
| image | 0.25.10 | MIT OR Apache-2.0、MITを選択、PNG/JPEGのみ |
| hound | 3.5.1 | Apache-2.0、原文を保持 |
| ab_glyph | 0.2.32 | Apache-2.0、原文を保持。Phase 5でシステムフォント描画に直接利用。既存のegui間接依存を再利用 |
| libc | 0.2.191 | MIT OR Apache-2.0、MITを選択。既存の間接依存を再利用し、Linuxの子プロセス終了制御に使用 |

正確な宣言と全175件の実ビルド対象は[THIRD_PARTY_LICENSES.md](../THIRD_PARTY_LICENSES.md)へ自動生成します。Cargo.lockの対象外プラットフォーム/未使用オプションのパッケージ数とは一致しません。Cargo metadataだけではオプション依存を過大に含むため、生成ツールは`cargo tree --target x86_64-unknown-linux-gnu -e normal,build,dev`で実対象を照合します。cargo-denyはそれとは独立してライセンス式を検査します。

Phase 5の文字画像生成にab_glyphを直接利用します。クレートのLICENSE原文を確認し、Apache-2.0として採用しています。ab_glyph_rasterizer/owned_ttf_parser/ttf-parser等の既存の間接依存も含め、実ビルド対象は175件で、許可リストの追加はありません。外部フォントを描画するだけで、フォントバイナリを実行ファイルやアセット付きプロジェクトへ取り込みません。カスタムフォントの利用条件は別途確認してください。

## 原文と著作権・NOTICE

`cargo run --locked --bin license-notices`で、crateアーカイブ中のLICENSE、COPYING、COPYRIGHT、NOTICE等を原文のまま`LICENSES/<crate>-<version>/`へ保存します。原文欠落時はエラーとなります。READMEのライセンス名だけで原文の代用はしません。

次の12クレートには必要な原文がアーカイブ内に無かったため、`.cargo_vcs_info.json`に記録された公開時コミットの上流リポジトリから補完しました。ecolor、eframe、egui、egui-winit、egui_glow、emath、epaint、fluent-langneg、gl_generator、intl_pluralrules、khronos_api、profilingです。取得元・コミット・ファイルSHA-256は[UPSTREAM_LICENSES.json](UPSTREAM_LICENSES.json)へ記録しています。通常のビルド・アプリ起動・再生成ではこれらの文書をネットから取得しません。

`gl_generator`と`khronos_api`ではソース冒頭の著作権表示も保持しました。KhronosのGL/GLX/WGLレジストリはApache-2.0、EGLレジストリの原文はMIT型の許諾文であることを確認し、`NOTICE-EMBEDDED.txt`へ収録しています。

Rust標準ライブラリの著作権一覧とMIT/Apache/Unicode等の原文も、使用ツールチェーンから別ディレクトリへ保存します。標準ライブラリ一覧は複数ターゲットの上位集合で、MPLのFortanix SGX ABIやEFI向け項目が文書に含まれることは、Linux GNU版へのリンクを意味しません。Rustコンパイラー、LLVM、rustdocや文書用フォントは配布しません。異なるRust版・ターゲット・crt-static等へ変更した場合は、この記録の流用で配布可と判断せず、改めて調査します。

## ビルドスクリプト・ネイティブ部分

[BUILD_SCRIPTS.md](BUILD_SCRIPTS.md)に実対象のスクリプト一覧を保存します。今回の確認では、主な処理はrustcの機能検出、cfg生成、同梱済みXML/プロトコルからのRustバインディング生成、pkg-configによるシステムライブラリ確認です。FFmpeg/VOICEVOXやその他の実行時コードをネットから取得する処理は確認されませんでした。

- glutin_egl_sys/glutin_glx_sysはgl_generator/Khronos定義からRustコードを生成します。EGL/GLX実装を取り込んでいません。
- wayland-backendのCログ変換shimは同クレートのMIT条件で提供されます。`log` feature有効時だけCコードをビルドする構成を確認しました。クレートのライセンス原文を保持します。
- smithay-client-toolkit/wayland-sys/x11-dlのスクリプトはシステム側のxkbcommon/Wayland/X11を確認します。システム共有ライブラリを配布物へコピーしません。
- PNG/JPEG処理・HTTP通信は今回のfeature構成ではRust実装であり、FFmpeg共有ライブラリやVOICEVOX COREへのFFIはありません。
- `libc::prctl(PR_SET_PDEATHSIG)`で、本アプリが生成したFFmpeg/ffprobe/試聴プロセスを親終了時に終了させます。OSのlibcを利用するだけで、追加のネイティブ実装を同梱しません。ユーザーが起動したENGINEには適用しません。

Releaseバイナリの`ldd`を検査し、直接の動的依存はlibgcc_s、libm、libc、Linuxローダーでした。`libavcodec`/`libavformat`/`libavutil`/VOICEVOX等への直接リンクはありません。OpenGL/EGL/GLX、Wayland、X11、xkbcommon等は必要時にシステムから動的ロードされるため、`ldd`だけでの監査にはしません。

OS側で確認したバージョンとローカル著作権資料は次のとおりです。これらは**非同梱のシステムコンポーネント**で、crate許可リストへ追加したものではありません。

| システム側 | 確認版 | 確認資料と扱い |
|---|---|---|
| libc6 | 2.43-2ubuntu2.4 | `/usr/share/doc/libc6/copyright`、LGPL等。OSの共有libcを動的利用、非同梱 |
| libgcc-s1 | 16-20260322-1ubuntu1 | GCCランタイムとRuntime Library Exception。OS共有ライブラリ、非同梱 |
| libwayland-client0 | 1.24.0-2 | `/usr/share/doc/libwayland-client0/copyright`、MIT/X11型、非同梱 |
| libx11-6 | 2:1.8.13-1ubuntu0.1 | `/usr/share/doc/libx11-6/copyright`、X11系の複数許諾文、非同梱 |
| libxkbcommon0 | 1.13.1-1 | `/usr/share/doc/libxkbcommon0/copyright`、MIT/X11/SGI等の原文、非同梱 |
| libegl1/libgl1/libglvnd0 | 1.7.0-3 | `/usr/share/doc/libglvnd0/copyright`、MIT/Apache等。ビルドツール/ソースの項目と実行時共有ライブラリを区別、非同梱 |
| libxcb1 | 1.17.0-2ubuntu1 | `/usr/share/doc/libxcb1/copyright`、非同梱 |
| libasound2t64 | 1.2.15.3-1ubuntu1.1 | `/usr/share/doc/libasound2t64/copyright`、LGPL-2.1以降。Phase 4の音声出力時だけ`libasound.so.2`を動的ロード、非同梱 |

Phase 4ではALSAのPCM APIを、既存の`libc`の`dlopen`/`dlsym`でシステムから読みます。これは仕様3.2のOS側システムコンポーネントとして個別確認したもので、LGPLクレートの採用や許可リストへの追加ではありません。ALSA本体、ヘッダー、プラグイン、実装コードをコピー・静的リンク・同梱しません。Rust製の最小ABI宣言と制御コードは独自実装です。音声をオフにすればALSAなしで映像を再生できます。

[ALSA公式COPYING](https://github.com/alsa-project/alsa-lib/blob/master/COPYING)および上記パッケージ原文を確認し、LGPL 2.1の6(b)にある、ユーザーのシステム上の共有ライブラリを利用し互換な変更版で動作する仕組みを使用します。固定の独自ライブラリ探索・署名検査等で差し替えを妨げず、MITコードの変更・再ビルドや、変更版ライブラリをデバッグするためのリバースエンジニアリングを制限しません。原文の[著作権表示](../LICENSES/system-alsa-lib/COPYRIGHT)と[LGPL 2.1](../LICENSES/system-alsa-lib/LGPL-2.1.txt)を保持し、アプリ内の「このアプリについて」と第三者表示へ利用を明記します。ALSAやプラグインを将来パッケージへ含める場合はこの非同梱の確認を流用せず、別途配布条件を監査します。

実際のGPUドライバーやディストリビューションごとの実装はOS側のものです。将来AppImage等を作成する場合にも、これらを自動収集して同梱してはならず、新たなネイティブ/配布物監査が必要です。

## フォント・画像・外部プログラム

eframeのdefault featuresとdefault_fontsを無効化し、glow/Wayland/X11だけを指定しています。epaint_default_fontsは実ビルド対象にありません。システムの日本語/欧文フォントを実行時に読み、フォントファイルを配布しません。公式ロゴ、キャラクター画像、外部素材、サンプル音声は同梱していません。

FFmpeg/ffprobeはユーザーが導入した外部プログラムです。検証に用いたFFmpeg 8.0.1-3ubuntu2はGPL有効のシステム版ですが、VoxMovieMakerへリンク・コピーしていません。VOICEVOX ENGINE 0.26.0はユーザーが既に起動したlocalhostサービスにHTTP接続して検証しました。ENGINE/CORE/モデルの取得、同梱、起動・停止はしていません。生成音声やその話者の規約はVoxMovieMakerのMITとは別です。

## 検査結果と公開前の残項目

- cargo-deny 0.20.2: `check licenses`成功。未使用のMIT-0/CC0許可についての警告のみで、拒否ライセンスはありません。
- 全175件のcrate原文、上流補完文書、Khronos著作権、Rust標準ライブラリ表示を保存する仕組みを用意しました。
- Native/FFI/ビルドスクリプトを別途確認しました。
- 公開用パッケージ自体は未作成です。配布直前のバイナリ/ファイル一覧と対象OSでの再検査、名称/商標確認、正式アプリケーションIDの決定が残っています。
- この監査は技術的な確認記録で、未作成の配布物や全利用法への法的判断を済ませたとするものではありません。

## 素材のファイル選択（2026-10-25追加）

素材の追加・再リンク時に、OSに導入済みの `zenity --file-selection` を独立した外部プロセスとして呼び出します。シェルは介さず、引数と標準出力でファイル名を受け渡します。Zenity・GTKの実行ファイル、共有ライブラリ、ソース、画像をリンク・コピー・同梱・自動導入しません。Zenityがない場合は独自のRust/eguiフォルダーブラウザーを使うため、実行時の必須依存ではありません。追加のCargo依存・許可ライセンスはありません。

[上流COPYING](https://github.com/GNOME/zenity/blob/master/COPYING)と、導入済み `/usr/share/doc/zenity/copyright` を確認しました。パッケージの記録ではソースヘッダーがLGPL-2以降、COPYING/READMEがLGPL-2.1以降で、差異は上流課題として記載されています。いずれもアプリへ組み込むクレートではなく、ユーザーのOS側で配布・管理される別プログラムとして扱います。将来の配布物にZenityやGTKを取り込む根拠にはせず、同梱する場合は改めて配布条件を監査する必要があります。

## PSD立ち絵（2026-11-06追加）

`apeireth-image-psd = 2.0.0-rc.5` を版固定・default features無効で採用しました。公開クレートの正規化Cargo.tomlは `Apache-2.0 OR MIT` を宣言し、Cargo.toml.origと各ソースは `Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors`、`MIT OR Apache-2.0` を保持しています。VMMではMITを選びます。親プロジェクトのライセンスのみを根拠にしていません。

crateアーカイブにはLICENSE全文がありませんでした。`.cargo_vcs_info.json` のコミット `16d0a680b9d25edf918e1ac36614d0c2efdf45bb` の `crates/engine/image/NOTICE.md` が `psd/` を明示してMIT OR Apache-2.0の適用範囲を定めることと、同階層の `LICENSE-MIT` の権利者・許諾文・免責を確認しました。MIT/Apache全文とNOTICEを変更せず `LICENSES/apeireth-image-psd-2.0.0-rc.5/upstream/` へ収録し、URL・コミット・SHA-256を `UPSTREAM_LICENSES.json` に記録しています。公開時メタデータは `dirty: true` なので、クレート内容がそのコミットと完全一致するという主張はしていません。実クレートのライセンス宣言・ソース表示と、PSDを含む範囲についての上流原文をそれぞれ確認しました。

MIT側の条件に従い、元の著作権表示と許諾全文を維持します。NOTICE中の他の画像クレート・フォント・ブランド・顔検出等の説明は原文保存のための上位集合で、それらをVMMへ取り込んだことを意味しません。PSDの実装や親プロジェクトからソース・画像・フォントをコピーせず、指定されたクレートだけをCargo経由で使用します。

通常依存は既に監査済みの `flate2` と `thiserror` です。`flate2` はdefault features無効の `rust_backend`、`thiserror` は2系で、解決済みの `miniz_oxide`・`adler2`・`simd-adler32`・`crc32fast`・proc-macro系も許可ライセンス内です。PSDクレート自身の `proptest` は同クレートの開発用依存で、VMMのビルドグラフには入りません。`serde`・`testgen`・`corpus` のオプションfeatureは無効、build scriptなし、追加のFFI・ネイティブライブラリ・バンドルアセットなしです。クレートは `unsafe_code = forbid` を使用しています。

実ビルド対象は176クレートです。`cargo-deny check licenses` は許可リストを増やさず成功し、`license-notices` は原文欠落0件で第三者一覧を生成しました。元PSDや立ち絵画像の権利はユーザーが各配布元の規約を確認する必要があり、VMMのMITライセンスの対象にはしません。テストPSDはRustで生成した単色の独自データのみです。FFmpeg・VOICEVOXの非同梱方針は維持しています。

このクレートの公開版rc.4/rc.5はいずれもRust 1.97を要求します。アプリのrust-versionとREADMEの最低要件を1.97へ更新し、実検証は既存環境のRust 1.99.1で行いました。1.97そのもの、任意の実PSD、配布パッケージの最終監査は未実施です。
