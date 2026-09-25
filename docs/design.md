# braintrain-tui 設計書

## コンセプト
「地頭が良くなる」は謳わない。fluid intelligenceへの転移効果は研究上怪しいため、
成績が可視化できる**個別スキルの訓練ミニゲーム集**として設計する。
N88BASIC風にTUI上で図形を描く体験を軸に据える。

## 対応スキルとミニゲーム(MVP範囲)

| ゲーム | 鍛える力 | 図形描画 |
|---|---|---|
| 図形回転判定 | 空間把握・メンタルローテーション | 要 |
| 鏡像判定 | 空間把握 | 要 |
| 反応速度(Stroop風・色と文字の不一致) | 抑制制御・反応速度 | 不要 |
| 暗算スピード | 処理速度 | 不要 |

いずれも出題→制限時間内に判定/回答→正誤とレイテンシを記録、を1問の単位とする。

- 1プレイは出題数固定(10問)で終了する
- 難易度は初級/中級/上級の固定段階から、プレイ前にメニューで手動選択する
- 操作は矢印キー/数字キーを押した瞬間に即答確定する。カーソル移動+Enter確定は行わない
  (2択ゲームは左右矢印キーがそのまま選択肢、4択の暗算は数字キー1〜4がそのまま選択肢)

## 全体アーキテクチャ

```
braintrain-tui/
  src/
    main.rs           # エントリ、terminal初期化、メインループ(tick駆動)
    app.rs            # App状態: Screen::{Menu, Playing(Box<dyn Game>), Result(GameResult), History}
    game/
      mod.rs           # trait Game
      shape_rotate.rs
      mirror_match.rs
      reaction.rs
      mental_calc.rs
    canvas/
      shapes.rs        # 図形定義(頂点リスト)・回転/鏡像アフィン変換
      renderer.rs       # Backend抽象: Braille描画 / (将来)Sixel・Kitty画像描画
    stats/
      store.rs          # JSONL永続化(1行1セッション記録)
      history_view.rs    # 成績推移グラフ(canvas折れ線)
  docs/
    design.md           # 本ファイル
```

### Game trait

```rust
pub trait Game {
    /// キー入力を受け取り、内部状態を更新する
    fn handle_key(&mut self, key: KeyEvent);
    /// tick駆動の更新(タイマー等)。経過時間を渡す
    fn update(&mut self, dt: Duration);
    /// 描画。Frame全体でなく割り当てられたRectのみ使う
    fn render(&self, frame: &mut Frame, area: Rect);
    /// このゲームのセッションが終わったか
    fn is_finished(&self) -> bool;
    /// 終了後にスコアを取り出す
    fn result(&self) -> GameResult;
}

pub struct GameResult {
    pub game_id: &'static str,
    pub correct: u32,
    pub total: u32,
    pub avg_latency_ms: f64,
    pub played_at: DateTime<Utc>,
}
```

各ミニゲームはこのtraitだけを実装し、`app.rs`のメインループは`Box<dyn Game>`を
差し替えるだけで済む。新しいミニゲームの追加は`game/`にファイルを1つ足すだけで
既存コードに影響しない(コンテキスト局所性)。

### 画面遷移

```
Menu ──(ゲーム選択)──> Playing ──(is_finished)──> Result ──> Menu
Menu ──(履歴選択)────> History ──> Menu
```

### 描画レイヤ(canvas/renderer.rs)

- MVPは`ratatui::widgets::canvas`のbraille方式のみを実装対象にする(依存追加ゼロ、
  ほぼ全端末で動く)
- 将来sixel/kitty画像プロトコル(`ratatui-image`等)を足す前提で、`renderer.rs`に
  `trait ShapeRenderer { fn draw(&self, shapes: &[Shape], area: Rect, frame: &mut Frame); }`
  を切っておき、braille実装(`BrailleRenderer`)を差し込む形にする。今回の対象は
  BrailleRendererのみで、sixel/kitty版は別実装として後日追加する(今は骨組みだけ
  用意し実装しない)

### 図形(canvas/shapes.rs)

- 図形は正規化座標系(-1.0〜1.0)の頂点リストとして定義する。3種類では出題がすぐ
  見切れて単調になったため、非対称で見分けやすい図形を8種類程度まで増やす
- 回転はアフィン変換(2D回転行列)、鏡像はX軸またはY軸反転
- 出題ロジック: 元図形と、回転/鏡像/別図形のいずれかを変換した図形を並べて表示し
  「同一か」を判定させる

### 永続化(stats/store.rs)

- 保存先: `~/.local/share/braintrain-tui/history.jsonl`(XDG準拠、`dirs`クレート)
- 1行1レコード、`GameResult`をそのままJSON化してappend
- 起動時に全件読み込みはせず、History画面表示時にのみ読み込む(規模が小さいツール
  なので素朴な実装で十分)

### 音声(audio/mod.rs)

- BGMとSE(正解/不正解/画面遷移音)を`rodio`で再生する
- `trait AudioPlayer { fn play_bgm(&self, track: BgmTrack); fn play_se(&self, se: SeKind); }`
  で抽象化し、ゲームロジック側はaudio実装の詳細に依存しない
- 音源ファイルは`assets/audio/`配下に置き、`rust-embed`でシングルバイナリに埋め込む
  (実行時にファイルパス解決不要、`cargo install`一発で音付きで動く)

### 依存クレート(想定)

- `ratatui` + `crossterm`: TUI本体
- `rand`: 出題生成
- `serde` / `serde_json`: GameResultのシリアライズ
- `chrono`: 日時
- `dirs`: 保存先パス解決
- `rodio`: BGM/SE再生

## 開発運用ルール(過去のvJ39プロジェクトからの反映)

- 各`Game`実装の`handle_key`/`update`が肥大化してきたら、その場でサブ関数やヘルパー
  モジュールに分割する。分割を先送りにしない
- BGM/SEを実装したら、こちらでの自動テストや起動確認だけで完了報告しない。ユーザー
  自身の実機で再生を確認してもらう一手間を挟む
- 見た目・体験のフィードバックは、具体的な指摘(どのゲームのどの画面のどこか)を得て
  から手を入れる

