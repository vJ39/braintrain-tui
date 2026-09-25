## 概要

DDR風リズムゲームで、現在は全ノーツの判定が終わった時点で`is_finished`がtrueになりリザルト画面へ進む。楽曲(BGM)がまだ再生中でも打ち切られる。曲が最後まで再生し終わってからリザルトに進むように変更する。

## 変更内容

`RhythmSong`構造体に、曲全体の長さ(`duration_ms: u32`)を実測値で追加する。

- `Top_of_the_Leaderboard.mp3`: 178808ms
- `Redline_Response_Time.mp3`: 179435ms
- `Apex_Movement.mp3`: 182204ms

`is_finished`の判定を、「全ノーツが判定済み」に加えて「`started_at.elapsed() >= song.duration_ms`」も満たすことを条件にする(両方満たしてtrue)。

## 表示

全ノーツ判定済みだが曲がまだ再生中の間は、現在の画面(トラック・判定ライン・フッター等)をそのまま表示し続ける(新規のノーツは無いので判定ラインより上には何も流れてこない)。

## スコープ外

- 譜面生成(`generate_chart`)・判定ロジックは変更しない
- BGM再生自体の開始・停止タイミング(`app.rs`側の`play_bgm_track`呼び出し)は変更しない
