# FUIDE — FUI Develop Environment

Sci-Fi / FUI (Futuristic UI) デザインのアプリを作るための開発環境。egui (0.36) 向けの `fuide` クレート (テーマ・窓シェル・部品・設定ウィンドウ・MCP エージェント) を開発するリポジトリ。アプリは別リポジトリに分かれている:

| リポジトリ | 中身 |
|---|---|
| [kobago/fuide-mac-utils](https://github.com/kobago/fuide-mac-utils) | FUIDE File Manager / Brew / Player / Activity Monitor (macOS ユーティリティ) |
| [kobago/fuide-3d](https://github.com/kobago/fuide-3d) | 3D ビューポート `fuide-3d` (wgpu、ホログラム描画) |
| [kobago/fuide-cad](https://github.com/kobago/fuide-cad) | FUIDE CAD |
| [kobago/fuide-eda](https://github.com/kobago/fuide-eda) | FUIDE EDA |
| [kobago/fuide-git-client](https://github.com/kobago/fuide-git-client) | FUIDE Git |
| [kobago/fuide-media-downloader](https://github.com/kobago/fuide-media-downloader) | FUIDE Media Downloader |

```
crates/fuide/        FUI 部品ライブラリ `fuide` (egui のみ依存)
  theme.rs           パレット CYAN / AMBER / GREEN、フォント登録、ウィジェットスタイル
  shell.rs           フレームレス窓シェル (直角の枠、グロー、タイトル/ステータスバー、リサイズ)
  panel.rs           タイトルチップ付きパネル
  widgets.rs         ナビタブ、ボタン、セグメントバー、円弧ゲージ、ランプ、ログフィード、読み出し行
  fx.rs              走査線、走査帯
  geom.rs            多角形、グロー描画 (チャンファーはオプション)
  pathinput.rs       パス入力欄の `~` / 相対パス展開と Tab 補完
  settings.rs        設定ファイルと設定ウィンドウ (パレット / 角 / 密度 / 透過 / AGENT)
  agent/             内蔵 MCP サーバー (Unix ソケット) と `--mcp` stdio ブリッジ
assets/fonts/        Orbitron (見出し) / Share Tech Mono (データ) — いずれも OFL
```

## 設定ウィンドウ (テーマ)

各アプリとも `Cmd+,` かタイトルバーの歯車で設定ウィンドウが開く。パレット (CYAN / AMBER / GREEN)、角 (SQUARE / CHAMFER)、密度 (NORMAL / COMPACT)、窓の透過 (WINDOW: TRANSLUCENT / OPAQUE。OPAQUE は本体の地色 `bg_deep` を不透明にしてデスクトップが透けないようにする。窓自体は透過のままなので、枠の外側のグローや面取りした角はこれまで通り抜ける)、AGENT (MCP サーバーの OFF / ON、確認ダイアログを HUMAN / AGENT のどちらが押すか。下の「AI エージェントから操作する」) を選ぶと即座に本体へ反映され、ファイルに保存される。閉じるのは × / Esc / Cmd+W。

- 設定ウィンドウは egui の **子 viewport** (別のネイティブウィンドウ、`show_viewport_deferred`) で、本体と同じ `fuide::Shell` を `tool_window()` (閉じるボタンのみ・リサイズなし・アイドルアニメ無し = 入力があったときだけ再描画) で描いている。フォントや Visuals は `egui::Context` 全体で共有なので、子ウィンドウで変えた瞬間に本体も変わる
- 子 viewport は eframe 0.36 では撮影できない (immediate は `Screenshot` コマンドを捨てる。deferred は macOS でイベントループが約 1 秒止まったあと再描画が来なくなる)。撮影は `FUIDE_DEV_EMBED=1` で本体に埋め込んで行う (各アプリの README の「開発用スクリーンショット」)
- 保存先は macOS では `~/Library/Application Support/FUIDE/<app>.conf` (`file-manager.conf` / `brew.conf` / `player.conf`)、他 OS では `$XDG_CONFIG_HOME/fuide/` か `~/.config/fuide/`。`FUIDE_CONFIG_DIR` で置き換え可。中身は `palette=amber` のような `key=value` 行 (`palette` / `chamfer` / `compact` / `transparent` / `agent` / `agent_confirm`、ログパネルをドラッグすると `log_height`、ログパネルを開閉すると `log_open`) で、知らないキーは無視、足りないキーは既定値
- 自作アプリで使うには `fuide::Settings` と `fuide::SettingsWindow` (下の「クレートの使い方」参照)

## AI エージェントから操作する (MCP)

各アプリは **MCP サーバー** を内蔵している。設定ウィンドウ (`Cmd+,`) の AGENT パネルで `ON` にすると Unix ソケットで待ち受け、Claude Code などの MCP クライアントが画面を読み・クリックし・文字を打てる。人が見ている前で AI が FUI を操作するための機能なので、操作は画面に見える形で行われる: エージェント用の照準カーソルが目標までなめらかに移動し、押した部品が光り、直前の操作 (`CLICK ▸ OUTDATED`) がカーソル脇に出る。ステータスバーには `AGENT` ランプが点く (操作中は点滅)。

```sh
# Claude Code に登録する例 (各アプリのリポジトリの README にアプリごとの行がある)
claude mcp add fuide-brew -- "/Applications/FUIDE Brew.app/Contents/MacOS/fuide-brew" --mcp
```

| ツール | 内容 |
|---|---|
| `observe` | アプリの状態要約 (表示中のビュー・選択・実行中のコマンド・ダイアログ・ログ末尾) と、画面上の操作できる部品の一覧 `[role] LABEL (state) @x,y`。最初に呼び、各操作のあとも返ってくる |
| `click {label, nth?}` | ラベルの部品へカーソルを動かしてクリック。ラベルは `observe` に出る文字列そのまま (大文字)。完全一致 → 大文字小文字無視 → 部分一致の順で探す |
| `type {text, label?, submit?}` | 入力欄に 1 文字ずつ打つ。`label` を付けるとその欄にフォーカスしてから。`submit` で最後に Enter |
| `key {key, repeat?}` | `enter` / `escape` / `down` / `cmd+3` / `cmd+f` / `cmd+backspace` など |
| `wait {ms}` | brew の実行やディレクトリ読込を待ってから観測を返す |
| `screenshot {scale?, path?}` | 窓を PNG で返す (画面収録権限は不要。`FUIDE_SCREENSHOT` と同じ自己撮影)。`path` を付けると保存もする |
| アプリ固有のツール | アプリが `Agent::set_tools` で足したもの (CAD の `add_feature` / `measure` など)。`tools/list` に並び、アプリ側で処理されて、結果の文の後に観測が付く。部品を押すわけではないので、代わりに**ツールが触った部品 (追加した行、書き換えた入力欄) へカーソルが飛んで光り**、脇に `TOOL ▸ ADD_FEATURE` と出る (`Agent::finish_tool` の `focus`)。`--mcp` ブリッジも同じ一覧を答える (`bridge::run_with_tools`) |

仕組みと決めごと:
- **クリックの注入**: egui の `Event::AccessKitActionRequest(Click)` を対象ウィジェットの id に向けて入れる。egui はこれを本物のクリックとして扱う (`Response::clicked()` が真になる) ので、座標を当てる必要がなく、部品が動いても壊れない。キーと文字は `Event::Key` / `Event::Text` で、`Cmd` などの修飾キーはそのフレームの `InputState::modifiers` に載せる
- **部品の一覧**: kit の部品は `Response::widget_info` の代わりに `fuide::agent::describe` を呼び、アクセシビリティ木への登録と同時にエージェント用の一覧にも載る (ラベル・種類・状態・矩形)。`egui::TextEdit` のように自前で木に載る部品は `fuide::agent::note` で一覧だけに足す。アプリ側は `agent_state()` で部品だけでは分からない状態を文章にして渡す
- **モーダル中は、ダイアログの部品しか操作できない** (前景レイヤーに部品があればそれだけを列挙する)。AccessKit 経由のクリックはモーダルの背後にも届いてしまうため
- **確認ダイアログの人間留保**: 設定の CONFIRM DIALOGS が `HUMAN` (既定) の間、brew の `UPGRADE` / `UNINSTALL` / `INSTALL`、ファイルマネージャーの `MOVE TO TRASH` / `DELETE PERMANENTLY` のボタンと Enter はエージェントに拒否され、観測に `(human only)` と出る。`CANCEL` は押せる。`AGENT` にすると自分で確定できる。リネームは可逆なので留保しない
- **通信**: アプリが `~/Library/Application Support/FUIDE/<app>.sock` (パスが長すぎるときは `$TMPDIR/fuide-<app>.sock`) で MCP (JSON-RPC 2.0、改行区切り) を話す。`<app> --mcp` は同じバイナリの stdio ブリッジで、`initialize` / `tools/list` は自分で答え、`tools/call` だけをソケットへ転送する。だから **Claude Code はアプリより先に起動していてよい**: 最初の呼び出しでアプリが無ければ `open -a` で起動して 12 秒待ち、AGENT が OFF なら「設定で ON にして」というエラーを返す
- 設定ウィンドウ (子 viewport) 自体はエージェントから操作できない (人間の操作面)。`FUIDE_DEV_EMBED=1` で本体に埋め込んだときは操作できる
- 依存は増やしていない: JSON は `serde_json`、PNG は macOS の `sips` で圧縮 (無ければ非圧縮 PNG を自前で書く)、base64 も自前

```sh
# 手で試す (nc は改行区切りの JSON-RPC をそのまま流せる)
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"observe","arguments":{}}}' \
  | nc -U ~/Library/Application\ Support/FUIDE/brew.sock
```

## テスト

```sh
cargo test                                   # 単体 + UI テスト (オフラインで完結、数秒)
UPDATE_SNAPSHOTS=true cargo test -p fuide    # 見た目が意図的に変わったときにスナップショットを更新
```

| 層 | 場所 | 中身 |
|---|---|---|
| 単体 (fuide) | 各モジュールの `#[cfg(test)]` | `fmt` / `fontmetrics` / `settings` の純関数 |
| UI (fuide) | `crates/fuide/tests/ui.rs` | [`egui_kittest`](https://docs.rs/egui_kittest) でシェル + パネル + 部品をヘッドレス描画。**アクセシビリティ木**でボタンやタブをラベルから探してクリック・状態確認、**wgpu スナップショット** (`tests/snapshots/*.png`、`kittest.toml` の閾値) で見た目の回帰を検出 |
| エージェント (fuide) | `crates/fuide/tests/agent.rs` | kittest 上で `Agent::submit` に `observe` / `click` / `type` / `key` / `screenshot` を流し、注入したクリックが kit の部品に届くこと、無効・人間留保・不明なラベルが拒否されること、PNG が返ることを検証 (ソケット無し) |

決めごと:
- 実機依存 (画面収録など) は `#[ignore]` か偽物に差し替え、`cargo test` はオフラインで通す
- **操作できる部品は必ず `Response::widget_info` でラベルを持つ** (`nav_tab` / `button` / `icon_button` / `toggle_chip` / テーブルの列見出し・行 / シェルの窓ボタンと歯車)。ラベルは**描画と同じ大文字**にする。アイコンだけのボタンは `Icon::label()` (`REFRESH` など)。これが UI テストと支援技術の共通の入口
- 状態は egui の流儀で読む: `WidgetInfo::selected` は AccessKit の `toggled` に写るので、テストでは `node.accesskit_node().toggled() == Some(Toggled::True)`
- シェルは常時アニメして毎フレーム再描画を要求するので、kittest では `run()` (静止待ち) ではなく `run_steps(n)` + `with_step_dt` で決定的に進める
- ハーネスは生成時に最初のフレームを回すため、フォント登録 (`theme::install`) は最初のフレームで行い、そのフレームは何も描かない (`set_fonts` は次パスから有効)
- ダイアログはフェードインの最初のフレーム (opacity 0) では部品が無効 (egui は不可視の `Ui` を disable する) なので、E2E では `!accesskit_node().is_disabled()` になるまで待ってからクリックする
- 同じ文字列が複数の場所に出るとき (選択した行の名前がインスペクターにも出る等) は `get_by_role_and_label(Role::Button, ..)` で絞る
- E2E が見つけた実バグ: egui は Esc でフォーカスを先に外すので `has_focus()` では Esc を拾えない → `lost_focus()` も見る (フィルターの Esc クリアが動いていなかった)。brew の検索ビューでは `Cmd+F` を検索欄に向ける


## 再描画レートとウィンドウマネージャー

シェルのアイドルアニメーション（枠のパルス・走査帯）は **20 fps** で再描画する（`fuide::shell::IDLE_FPS`、環境変数 `FUIDE_IDLE_FPS` で変更、`0` = 毎フレーム）。毎フレーム再描画すると macOS では Rectangle などのスナップ操作で 200〜500 ms 遅れる（[winit #3644](https://github.com/rust-windowing/winit/issues/3644)、[kobago/fuide#1](https://github.com/kobago/fuide/issues/1)）。ダイアログのフェードや brew 出力の流入など一時的なアニメーションは従来どおり即時に再描画する。`FUIDE_DEV_FRAMELOG=1` で 30 フレームごとの時刻を stderr に出せる。

## 他のプロジェクトから `fuide` を使う (git 依存)

`fuide` はまだ crates.io には公開していないので、GitHub の URL を `Cargo.toml` に書いて取り込む。ワークスペース内の `crates/fuide` は Cargo がパッケージ名で見つけるので、パスの指定は不要。

```toml
[dependencies]
egui = "0.36.1"
eframe = { version = "0.36.1", default-features = false, features = ["default_fonts", "wgpu"] }
fuide = { git = "https://github.com/kobago/fuide" }
```

- 再現性のため、コミットかタグで固定するのを推奨: `{ git = "...", rev = "65ca5ba" }` / `{ git = "...", tag = "v0.1.0" }`。`branch = "main"` で追従もできる。何も書かなくても `Cargo.lock` にコミットが記録され、`cargo update` で進む
- リポジトリが private の間は認証が要る。SSH が簡単: `fuide = { git = "ssh://git@github.com/kobago/fuide" }`。HTTPS を使うなら `~/.cargo/config.toml` に `[net] git-fetch-with-cli = true` を入れてシステムの git (credential helper) に任せる
- `egui` / `eframe` は `fuide` と同じ 0.36 系に揃える (ずれると型が一致せずコンパイルできない)
- crates.io に公開したら `fuide = "0.1"` に差し替えるだけで移行できる

## `fuide` クレートの使い方 (最小)

```rust
fn new(cc: &eframe::CreationContext<'_>) -> Self {
    fuide::theme::install(&cc.egui_ctx, fuide::Palette::cyan(), vec![]);
    ..
}
fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
    fuide::Shell::new("My Tool").subtitle("v0.1").lamp("LINK OK", pal.ok, false)
        .show(ui, |ui| {
            fuide::Panel::new("Telemetry").show_rect(ui, rect, |ui| { .. });
        });
}
```

`NativeOptions.viewport` は `with_decorations(false).with_transparent(true)`、`App::clear_color` は `[0.0; 4]` にする (設定の WINDOW = OPAQUE でも窓は透過のまま。`Settings::apply` がパレットの `bg_deep` を `Palette::opaque` で不透明にして本体を塗りつぶす)。`Shell` は Cmd+W で自分のウィンドウに `ViewportCommand::Close` を送る (本体なら終了、`tool_window()` は自前で閉じる)。

設定ウィンドウを付けるなら、起動時に `Settings::load("my-tool")` で読んで `install` に渡し、毎フレームの最後に `SettingsWindow::show` を呼ぶ:

```rust
let settings = fuide::Settings::load("my-tool").unwrap_or_else(|| fuide::Settings::new(fuide::PaletteKind::Cyan));
fuide::theme::install(&cc.egui_ctx, settings.palette.palette(), vec![]);
settings.apply(&cc.egui_ctx);
..
let out = fuide::Shell::new("My Tool").settings_button(true).show_full(ui, |ui| { .. });
if out.settings_clicked { self.settings_win.open(); }
if self.settings_win.show(ui.ctx(), &mut self.settings, "My Tool") {
    self.settings.save("my-tool").ok();   // 変更があったフレームだけ true
}
```

文字サイズは `fuide::TypeScale` に集約 (既定 `NORMAL`: 本文 13.5px / ラベル・見出し 13px / 脚注 12px / 行高 24px、Finder の 13px 相当)。密度を上げたいときは `theme::set_type_scale(&ctx, TypeScale::COMPACT)` か `.scaled(f)`。egui 標準の Cmd +/- でも全体をズームできる。

角は既定で直角。45° のチャンファーが欲しいときだけ `fuide::theme::set_corners(&ctx, fuide::Corners::CHAMFER)` を呼ぶ (窓 26 / パネル 14 / タブ 12 / ボタン 7 px)。
