## 概要

TTR(リズムゲーム)に新曲「Overclocked_Tempo」を追加し、あわせてTTR全体の難易度選択を廃止する(常に上級譜面・上級判定でプレイする)。既存の初級/中級用のコード・テストは削除する。

## 対象ファイル

- `src/game/rhythm.rs`
- `src/app.rs`

## 1. Overclocked_Tempoの曲データ追加

`src/game/rhythm/beats.rs` には既に `pub(super) const OVERCLOCKED_TEMPO_BEATS_MS: &[u32]`(433個、実測ビート時刻ms)を追加済み。これはそのまま使う(解析済みなので変更不要)。

`src/game/rhythm.rs` の `SONGS` 配列に、以下の内容で曲を追加する。

- `track_name: "Overclocked_Tempo"`(`assets/audio/bgm/rhythm/`に配置済み。既存の`BgmCategory::Rhythm`が指すディレクトリと同じ)
- `display_name: "Overclocked Tempo (BPM 152)"`
- `duration_ms: 177_476`
- `beat_times_ms: beats::OVERCLOCKED_TEMPO_BEATS_MS`
- セクション定義(新規定数 `OVERCLOCKED_TEMPO_SECTIONS`。既存の `TOP_OF_THE_LEADERBOARD_SECTIONS` 等と同じパターンで、`beat_index_at_or_after_secs` を使って秒指定からビートインデックスへ変換する):

```
use SectionDensity::{Extreme, High, Low, Mid};
const B: &[u32] = beats::OVERCLOCKED_TEMPO_BEATS_MS;
&[
    (beat_index_at_or_after_secs(B, 0), Low),
    (beat_index_at_or_after_secs(B, 10), Mid),
    (beat_index_at_or_after_secs(B, 50), Low),
    (beat_index_at_or_after_secs(B, 60), Mid),
    (beat_index_at_or_after_secs(B, 90), Extreme),
    (beat_index_at_or_after_secs(B, 115), Mid),
    (beat_index_at_or_after_secs(B, 140), Low),
    (beat_index_at_or_after_secs(B, 145), High),
    (beat_index_at_or_after_secs(B, 150), Extreme),
    (beat_index_at_or_after_secs(B, 170), Low),
]
```

(実測RMSエネルギー解析による大まかな盛り上がり区分。0-10秒イントロ、10-50秒盛り上がり、50-60秒ブレイク、60-90秒再上昇、90-115秒最高潮、115-140秒落ち着き、140-145秒ブレイク、145-150秒再構築、150-170秒最高潮、170秒以降終息)

Overclocked_Tempoは曲一覧の最後(既存3曲の後)に追加する。

## 2. TTR全体を「難易度選択なし・常に上級相当」にする

### rhythm.rsの変更

- `judge_windows(difficulty: Difficulty) -> JudgeWindows` 関数を削除し、Advancedの値(`perfect: 35ms, great: 70ms, good: 120ms`)を使う形にする(関数を引数なしにする、または`JudgeWindows`の定数を直接持つ、実装しやすい形でよい)
- `step_pattern_for(density, difficulty, beat_index)` から `difficulty` 引数を削除し、既存の `(Difficulty::Advanced, ...)` の分岐だけを残す(Beginner/Intermediateの分岐は削除)
- `generate_chart(song: &RhythmSong, difficulty: Difficulty) -> Vec<Note>` から `difficulty` 引数を削除する
- `RhythmGame` 構造体の `difficulty` フィールドと `RhythmGame::new(difficulty: Difficulty, song_index: usize)` の `difficulty` 引数を削除する(常に旧Advanced相当の譜面・判定になる)
- `Difficulty` 型自体は他のゲーム(count_mania等)で使われ続けるので削除しない。rhythm.rs内で使わなくなるだけ
- Beginner/Intermediate関連の既存テスト(`step_pattern_beginner_*`, `step_pattern_intermediate_*`, `ALL_DIFFICULTIES`を使うテスト等)は削除する。Advanced関連のテストは、difficulty引数を取らない形に書き換えて残す
- `SectionDensity::Extreme`(旧「Advanced専用」のコメント)は、難易度分岐が無くなるため「最高難度区間」という説明に更新する

### app.rsの変更

現状、TTRも他のゲームと同じ`Screen::SelectDifficulty(item, song)`画面を経由して難易度を選んでからプレイを始めている(`select_song`→`SelectDifficulty(RHYTHM_ITEM_INDEX, Some(song))`→難易度選択→`start_playing`→`start_rhythm(song, difficulty)`)。TTRだけはこの難易度選択画面を経由せず、曲を選んだら即座にプレイを開始するようにする。

- `select_song(song: usize)` を、選んだ曲がTTR(常にTTR、他ゲームはこの関数を使わない)であることを踏まえて、`SelectDifficulty`へ遷移せず直接 `start_rhythm(song)` を呼ぶように変更する
- `start_rhythm(song: usize, difficulty: Difficulty)` から `difficulty` 引数を削除する(呼び出し元は上記の通り`select_song`のみになるはず。既存の`start_playing`内の`if item == RHYTHM_ITEM_INDEX { ...; self.start_rhythm(song.unwrap_or(0), difficulty); return; }`分岐は、TTRがもう`SelectDifficulty`画面を経由しなくなるため到達しなくなる。到達しなくなったコードは削除する)
- `Screen::SelectDifficulty(item, song)` 自体は他のゲームで引き続き使うので型は変更しない。TTRを選んだ時にこの画面へ遷移する経路が無くなるだけ
- 関連する既存テスト(`ttr_splash_bgm_keeps_playing_through_difficulty_select`等、`SelectDifficulty(RHYTHM_ITEM_INDEX, ...)`を前提にしているテスト)を、「曲を選んだら直接プレイが始まる」新しいフローに合わせて更新する
- 画面フローは「TOP → メニューでTTR選択(ここでBGM開始) → TTRスプラッシュ → 曲選択 → (難易度選択を挟まず)直接プレイ開始」になる

## 3. TTRスプラッシュ画面と曲選択画面の統合

現状、TTR選択後は `Screen::RhythmSplash`(画像1枚・Enter/クリック待ちのスプラッシュ画面)→`Screen::SelectSong(usize)`(曲リストの選択画面)の2画面を経由している。これを1画面に統合し、TTRスプラッシュ画像を背景にしたまま曲を選べるようにする。

- `Screen::RhythmSplash` バリアントと、それに関連する `leave_rhythm_splash`・`Screen::RhythmSplash`分岐のキー/マウス処理を削除する
- `select_menu_item` のTTR分岐(`RHYTHM_ITEM_INDEX`)は、`Screen::RhythmSplash` ではなく `Screen::SelectSong(0)` へ直接遷移するようにする(TTR専用BGM(`BgmCategory::RhythmSplash`)の再生開始はそのまま維持する)
- `render_song_select` の描画で、`app.ttr_splash_renderer`(既存の `SplashRenderer`。画像プロトコル対応/フォールバックの両方を持つ)を背景として全画面に描画してから、その上に既存の曲選択パネル(`theme::panel`のブロックと曲リスト)を重ねて描画する
- 曲選択画面のキー操作(↑↓・Enter・数字)・マウスクリックは既存のまま(`Screen::SelectSong`のロジックは変えない)
- 画面フローは最終的に「TOP → メニューでTTR選択(BGM開始) → (スプラッシュ画像を背景にした)曲選択画面 → 曲を決定 → 直接プレイ開始(上級固定)」になる

### 影響するテスト

- `selecting_rhythm_menu_item_enters_ttr_splash_first` 等、`Screen::RhythmSplash` を前提にしたテストは、`Screen::SelectSong(0)` へ直接遷移することを確認する形に書き換える
- `ttr_splash_enter_key_goes_to_song_select`・`ttr_splash_click_goes_to_song_select`・`ttr_splash_other_key_stays_on_ttr_splash`・`ttr_splash_q_returns_to_menu`・`ttr_splash_renders_without_panicking`・`ttr_splash_uses_its_own_renderer_not_the_title_one` は、統合後の画面(`Screen::SelectSong`)に対する動作(qでメニューに戻る、背景にTTR画像が使われる等)として書き換えるか、不要なら削除する
- `ttr_splash_bgm_keeps_playing_through_song_select`・`ttr_splash_bgm_keeps_playing_through_difficulty_select` は、統合後もBGMが途切れないことを確認する形で残す

## テスト観点

- Overclocked_Tempoの曲データがSONGSに含まれ、`assets/audio/bgm/rhythm/Overclocked_Tempo.mp3` が存在すること(既存の `songs_track_names_exist_in_rhythm_bgm_assets` 相当のテストで検証できる)
- セクション区間が昇順であること(既存の `songs_sections_start_at_zero_and_are_sorted` 相当のテストがOverclocked_Tempoにも適用されること)
- `RhythmGame::new(song_index)` がdifficulty引数なしで動作し、生成される譜面が旧Advanced相当の密度になること
- 判定ウィンドウが旧Advancedの値(perfect35/great70/good120)であること
- app.rsで、TTR選択→スプラッシュ→曲選択のあと、難易度選択画面を経由せず直接`Screen::Playing`になること
- 曲選択画面のマウスクリック・キー操作(Enter)の両方で、直接プレイが始まること
- 他のゲーム(count_mania等)の`SelectDifficulty`画面の動作に影響が無いこと
