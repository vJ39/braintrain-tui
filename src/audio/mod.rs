use std::cell::{Cell, RefCell};
use std::io::Cursor;

use std::time::Duration;

use rand::Rng;
use rodio::source::SineWave;
use rodio::{OutputStream, OutputStreamHandle, Sink, Source};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/audio/"]
struct Assets;

/// assets/audio/bgm/ 配下は用途別にサブフォルダで分ける
/// (menu/ = 起動画面・Playing以外、playing/ = ゲームプレイ中、
///  rhythm/ = リズムゲームの楽曲。譜面と同期させるため曲ごとに選んで再生する、
///  rhythm_splash/ = TTR専用スプラッシュ画面〜曲選択画面、
///  beigoma_splash/ = 「べー」開始前のスプラッシュ画面、
///  result/ = ゲーム終了後のリザルト画面)
#[derive(RustEmbed)]
#[folder = "assets/audio/bgm/"]
struct BgmAssets;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeKind {
    Correct,
    Incorrect,
    Transition,
    /// タイトル画面(Splash)でEnter/クリックした時の決定音
    Confirm,
    /// 「べー」でベーゴマが盤外に吹っ飛んだ時の「ふいっ」という星の音
    Star,
    /// カウントダウン(3.2.1.GO!!)の「GO!!」の音
    CountdownGo,
    /// 「べー」でベーゴマが凹にはまった時の音
    BeigomaSink,
    /// 「べー」でベーゴマが凸に触れた時の音
    BeigomaBump,
    /// 「べー」でベーゴマが場外に落ちた時の音
    BeigomaFalloff,
    /// 「べー」でベーゴマがゴール旗に到着した時の音
    BeigomaGoal,
    /// 「ハヤウチ」で「まだ撃つな」が出た瞬間の構え(準備)の音
    QuickDrawReady,
    /// 「ハヤウチ」で合図に反応して撃った時の音
    QuickDrawShoot,
    /// 「ハヤウチ」でフェイント(「撃つな」)にひっかかって撃ってしまった時の音
    QuickDrawMiss,
    /// メニューでゲームの項目を選択(決定)した時の音
    GameSelect,
    /// TTRの曲選択画面で曲を決定した時の音
    TtrSongSelect,
    /// 「カウントメニア」で数字をクリックして正解した時の音
    CountManiaPop,
    /// メニュー等でカーソル移動(選択項目の変更)をした時の音
    CursorMove,
    /// 「シタケシ」でブロックを消した時の音
    ColorStackClear,
    /// 「シタケシ」で間違えて消えなかった時の音
    ColorStackMiss,
    /// 「やっほー」で相手が「やっほー」と言う時の音声(複数候補からランダムに1つ)
    LookAwayYahho,
    /// 「やっほー」で相手が「ヤー!」と叫ぶ時の音声(複数候補からランダムに1つ)
    LookAwayShout,
    /// 「べー」で軽トラが障害物を避ける・停止する・発進する時のスキール音(複数候補からランダムに1つ)
    BeigomaSkid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgmCategory {
    Menu,
    Playing,
    /// ゲーム終了後のリザルト画面(成功・GAME OVER以外)
    Result,
    /// ゲーム終了後のリザルト画面(GAME OVER、1問も正解できずに終わった時)
    ResultFailure,
    /// リズムゲームの楽曲。再生は譜面と対応する曲をトラック名で直接指定するため
    /// ランダム選曲には使わず、曲データとassetsの対応確認(テスト)で参照する
    #[cfg_attr(not(test), allow(dead_code))]
    Rhythm,
    /// TTR専用スプラッシュ画面〜曲選択画面の間に流す専用BGM
    RhythmSplash,
    /// 「べー」開始前のスプラッシュ画面(Screen::BeigomaSplash)に流す専用BGM
    BeigomaSplash,
    /// 「べー」のゲーム本編中に流す専用BGM
    BeigomaPlaying,
    /// 「ヤッホー」のゲーム本編中に流す専用BGM(演歌)
    LookAway,
}

impl BgmCategory {
    fn dir_prefix(self) -> &'static str {
        match self {
            BgmCategory::Menu => "menu/",
            BgmCategory::Playing => "playing/",
            BgmCategory::Result => "result/",
            BgmCategory::ResultFailure => "result_failure/",
            BgmCategory::Rhythm => "rhythm/",
            BgmCategory::RhythmSplash => "rhythm_splash/",
            BgmCategory::BeigomaSplash => "beigoma_splash/",
            BgmCategory::BeigomaPlaying => "beigoma_playing/",
            BgmCategory::LookAway => "look_away/",
        }
    }
}

/// パス(例: "menu/Calculated_Play.mp3")からディレクトリと拡張子を除いた
/// トラック名("Calculated_Play")を求める
fn track_name_from_path(path: &str) -> String {
    let file_name = path.rsplit_once('/').map(|(_, name)| name).unwrap_or(path);
    file_name
        .rsplit_once('.')
        .map(|(stem, _)| stem.to_string())
        .unwrap_or_else(|| file_name.to_string())
}

/// 指定カテゴリのBGMトラック名一覧(名前順)
pub fn bgm_tracks_in(category: BgmCategory) -> Vec<String> {
    let prefix = category.dir_prefix();
    let mut names: Vec<String> = BgmAssets::iter()
        .filter(|p| p.starts_with(prefix))
        .map(|p| track_name_from_path(&p))
        .collect();
    names.sort();
    names
}

/// ジュークボックスに表示する全カテゴリ分のBGMトラック名一覧(名前順、重複なし)
pub fn bgm_track_names() -> Vec<String> {
    let mut names: Vec<String> = BgmAssets::iter().map(|p| track_name_from_path(&p)).collect();
    names.sort();
    names.dedup();
    names
}

/// 指定カテゴリからランダムに1曲選ぶ。そのカテゴリに曲が無ければNone
pub fn random_bgm_track(category: BgmCategory) -> Option<String> {
    let tracks = bgm_tracks_in(category);
    if tracks.is_empty() {
        return None;
    }
    let index = rand::thread_rng().gen_range(0..tracks.len());
    Some(tracks[index].clone())
}

/// 「やっほー」で相手が「やっほー」と言う時の音声の候補(1つをランダムに再生)
const LOOK_AWAY_YAHHO_VOICES: [&str; 4] = [
    "voice_look_away_yahho_1.mp3",
    "voice_look_away_yahho_2.mp3",
    "voice_look_away_yahho_3.mp3",
    "voice_look_away_yahho_4.mp3",
];

/// 「やっほー」で相手が「ヤー!」と叫ぶ時の音声の候補(1つをランダムに再生)
const LOOK_AWAY_SHOUT_VOICES: [&str; 2] = [
    "voice_look_away_shout_1.mp3",
    "voice_look_away_shout_2.mp3",
];

/// 「べー」で軽トラのスキール音の候補(1つをランダムに再生)
const BEIGOMA_SKID_VOICES: [&str; 4] = [
    "voice_beigoma_skid_1.mp3",
    "voice_beigoma_skid_2.mp3",
    "voice_beigoma_skid_3.mp3",
    "voice_beigoma_skid_4.mp3",
];

impl SeKind {
    /// 音源ファイルのパス。合成音(Incorrect・Star)と、複数候補からランダムに選ぶ音声
    /// (LookAwayYahho・LookAwayShout・BeigomaSkid)はここでは決まらないのでNone
    fn asset_path(self) -> Option<&'static str> {
        match self {
            SeKind::Correct => Some("se_correct.wav"),
            SeKind::Incorrect => None,
            SeKind::Transition => Some("se_transition.wav"),
            SeKind::Confirm => Some("se_confirm.mp3"),
            SeKind::Star => None,
            SeKind::CountdownGo => Some("se_countdown_go.mp3"),
            SeKind::BeigomaSink => Some("se_beigoma_sink.mp3"),
            SeKind::BeigomaBump => Some("se_beigoma_bump.mp3"),
            SeKind::BeigomaFalloff => Some("se_beigoma_falloff.mp3"),
            SeKind::BeigomaGoal => Some("se_beigoma_goal.mp3"),
            SeKind::QuickDrawReady => Some("se_hayauchi_ready.mp3"),
            SeKind::QuickDrawShoot => Some("se_hayauchi_shoot.mp3"),
            SeKind::QuickDrawMiss => Some("se_hayauchi_miss.mp3"),
            SeKind::GameSelect => Some("se_game_select.mp3"),
            SeKind::TtrSongSelect => Some("se_ttr_song_select.mp3"),
            SeKind::CountManiaPop => Some("se_count_mania_pop.mp3"),
            SeKind::CursorMove => Some("se_cursor_move.mp3"),
            SeKind::ColorStackClear => Some("se_color_stack_clear.mp3"),
            SeKind::ColorStackMiss => Some("se_color_stack_miss.mp3"),
            SeKind::LookAwayYahho | SeKind::LookAwayShout | SeKind::BeigomaSkid => None,
        }
    }

    /// 複数候補からランダムに1つ選んで鳴らす音声の候補一覧(対象外はNone)
    fn voice_candidates(self) -> Option<&'static [&'static str]> {
        match self {
            SeKind::LookAwayYahho => Some(&LOOK_AWAY_YAHHO_VOICES),
            SeKind::BeigomaSkid => Some(&BEIGOMA_SKID_VOICES),
            SeKind::LookAwayShout => Some(&LOOK_AWAY_SHOUT_VOICES),
            _ => None,
        }
    }
}

/// tick音(TTRの最初の1小節で鳴らすカウント音)の周波数
const TICK_FREQUENCY_HZ: f32 = 880.0;
/// tick音の長さ
const TICK_DURATION: Duration = Duration::from_millis(60);
/// tick音の音量(1.0=元の振幅)。音源ファイルそのままの音量で鳴らすSEより控えめにする
const TICK_VOLUME: f32 = 0.2;

/// tick音の波形。音源ファイルを使わず、サイン波を短く切り出して生成する
fn tick_source() -> impl Source<Item = f32> {
    SineWave::new(TICK_FREQUENCY_HZ)
        .take_duration(TICK_DURATION)
        .amplify(TICK_VOLUME)
}

/// 不正解ブザー音の周波数。低めにして耳障りな「ブー」という音にする
const BUZZ_FREQUENCY_HZ: f32 = 180.0;
/// 不正解ブザー音の長さ
const BUZZ_DURATION: Duration = Duration::from_millis(350);
/// 不正解ブザー音の音量(1.0=最大振幅)
const BUZZ_VOLUME: f32 = 0.25;
/// 不正解ブザー音のサンプリングレート
const BUZZ_SAMPLE_RATE: u32 = 44100;

/// 不正解ブザー音の波形。サイン波ではなく矩形波(方形波)にすることで、
/// 元のse_incorrect.wav(ブブー)よりもブザーらしい耳障りな音にする。
/// rodioには矩形波のSourceが用意されていないため、波形を直接計算してSamplesBufferにする
fn buzz_source() -> impl Source<Item = f32> {
    let total_samples =
        (BUZZ_SAMPLE_RATE as f64 * BUZZ_DURATION.as_secs_f64()).round() as usize;
    let samples: Vec<f32> = (0..total_samples)
        .map(|i| {
            let phase = (i as f32 / BUZZ_SAMPLE_RATE as f32 * BUZZ_FREQUENCY_HZ).fract();
            let square = if phase < 0.5 { 1.0 } else { -1.0 };
            square * BUZZ_VOLUME
        })
        .collect();
    rodio::buffer::SamplesBuffer::new(1, BUZZ_SAMPLE_RATE, samples)
}

/// 「ふいっ」音のサンプリングレート
const WHOOSH_SAMPLE_RATE: u32 = 44100;
/// 立ち上がり(アタック)の時間。定常音にならないよう、すぐに減衰へ移る
const WHOOSH_ATTACK_SECS: f32 = 0.003;
/// 「ふいっ」の各音(start_freq, end_freq, duration_ms, decay, volume, delay_ms)。
/// 高い音から低い音へ一瞬で抜ける下降スイープ+速い減衰で、
/// 何かがすっと消え去る「ふいっ」という質感にする
const WHOOSH_TONES: [(f32, f32, u64, f32, f32, u64); 1] = [(2200.0, 300.0, 110, 18.0, 0.24, 0)];

/// 1音ぶんの波形をbufferのdelay_ms位置から加算する(周波数は指数補間でスイープさせ、
/// アタック→指数減衰のエンベロープをかける)
fn add_whoosh_tone(
    buffer: &mut [f32],
    start_freq: f32,
    end_freq: f32,
    duration_ms: u64,
    decay: f32,
    volume: f32,
    delay_ms: u64,
) {
    let delay_samples = (WHOOSH_SAMPLE_RATE as u64 * delay_ms / 1000) as usize;
    let tone_samples = (WHOOSH_SAMPLE_RATE as u64 * duration_ms / 1000).max(1) as usize;
    let ratio = (end_freq / start_freq).max(1e-6);
    let mut phase = 0.0f32;
    for i in 0..tone_samples {
        let t = i as f32 / WHOOSH_SAMPLE_RATE as f32;
        let progress = (i as f32 / tone_samples as f32).clamp(0.0, 1.0);
        let freq_now = start_freq * ratio.powf(progress);
        phase += 2.0 * std::f32::consts::PI * freq_now / WHOOSH_SAMPLE_RATE as f32;
        if phase > 2.0 * std::f32::consts::PI {
            phase -= 2.0 * std::f32::consts::PI;
        }
        let envelope = if t < WHOOSH_ATTACK_SECS {
            t / WHOOSH_ATTACK_SECS
        } else {
            (-decay * (t - WHOOSH_ATTACK_SECS)).exp()
        };
        if let Some(sample) = buffer.get_mut(delay_samples + i) {
            *sample += phase.sin() * envelope * volume;
        }
    }
}

/// 「ふいっ」という、何かがすっと消え去る音。「べー」でベーゴマが盤外に吹っ飛んだ時に鳴らす
fn whoosh_source() -> impl Source<Item = f32> {
    let total_ms = WHOOSH_TONES
        .iter()
        .map(|&(_, _, duration_ms, _, _, delay_ms)| duration_ms + delay_ms)
        .max()
        .unwrap_or(0);
    let total_samples = (WHOOSH_SAMPLE_RATE as u64 * total_ms / 1000).max(1) as usize;
    let mut samples = vec![0.0f32; total_samples];
    for &(start_freq, end_freq, duration_ms, decay, volume, delay_ms) in &WHOOSH_TONES {
        add_whoosh_tone(
            &mut samples,
            start_freq,
            end_freq,
            duration_ms,
            decay,
            volume,
            delay_ms,
        );
    }
    rodio::buffer::SamplesBuffer::new(1, WHOOSH_SAMPLE_RATE, samples)
}

/// タイプライター演出で文字が流れている間ループ再生するSEの種類。画面ごとに音源を変える
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypewriterSeKind {
    /// メニュー画面の説明文表示
    Menu,
    /// リザルト画面の表示
    Result,
}

impl TypewriterSeKind {
    fn asset_path(self) -> &'static str {
        match self {
            TypewriterSeKind::Menu => "se_typewriter.mp3",
            TypewriterSeKind::Result => "se_typewriter_result.mp3",
        }
    }
}

/// タイプライターSEをデコードし、終わりなくループさせた音源。音源が無い/デコードできなければNone
fn typewriter_loop_source(kind: TypewriterSeKind) -> Option<impl Source<Item = i16>> {
    let file = Assets::get(kind.asset_path())?;
    let source = rodio::Decoder::new(Cursor::new(file.data.into_owned())).ok()?;
    Some(source.repeat_infinite())
}

/// 実際にrodioで音声デバイスへ再生するプレイヤー。
/// 音声デバイスが無い/取得できない環境では初期化時にNoneとなり、以後は何もしない。
pub struct RodioPlayer {
    handle: Option<(OutputStream, OutputStreamHandle)>,
    /// 現在ループ再生しているBGMのSink。次の曲に切り替える/停止するとき使う
    bgm_sink: RefCell<Option<Sink>>,
    /// タイプライターSEをループ再生しているSink。BGMとは別に持ち、互いの再生・停止に影響させない
    typewriter_sink: RefCell<Option<Sink>>,
    /// タイプライターSEのループ再生中の種類。音声デバイスの有無に関わらず、再生を始めてから
    /// 止めるまでSomeにする(デバイスが無くSinkを作れない環境でも状態を確かめられるように)
    typewriter_looping: Cell<Option<TypewriterSeKind>>,
}

impl RodioPlayer {
    pub fn new() -> Self {
        let handle = OutputStream::try_default().ok();
        Self {
            handle,
            bgm_sink: RefCell::new(None),
            typewriter_sink: RefCell::new(None),
            typewriter_looping: Cell::new(None),
        }
    }

    /// 音声デバイスを使わないプレイヤー(テスト用)
    #[cfg(test)]
    fn without_device() -> Self {
        Self {
            handle: None,
            bgm_sink: RefCell::new(None),
            typewriter_sink: RefCell::new(None),
            typewriter_looping: Cell::new(None),
        }
    }

    fn play_se(&self, se: SeKind) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        // 複数候補からランダムに1つ選んで鳴らす音声
        if let Some(candidates) = se.voice_candidates() {
            let index = rand::thread_rng().gen_range(0..candidates.len());
            if let Some(file) = Assets::get(candidates[index]) {
                if let Ok(source) = rodio::Decoder::new(Cursor::new(file.data.into_owned())) {
                    sink.append(source);
                    sink.detach();
                }
            }
            return;
        }
        // 合成音(音源ファイルを使わない)は種類ごとの生成関数で鳴らす
        let Some(path) = se.asset_path() else {
            match se {
                SeKind::Incorrect => sink.append(buzz_source()),
                SeKind::Star => sink.append(whoosh_source()),
                _ => return,
            }
            sink.detach();
            return;
        };
        let Some(file) = Assets::get(path) else {
            return;
        };
        if let Ok(source) = rodio::Decoder::new(Cursor::new(file.data.into_owned())) {
            sink.append(source);
            sink.detach();
        }
    }

    /// 生成したtick音を1回鳴らす。BGMとは別のSinkで鳴らすので、再生中のBGMは止めない
    fn play_tick(&self) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        sink.append(tick_source());
        sink.detach();
    }

    /// 指定トラックをループ再生する。既に再生中のBGMがあれば止めて切り替える
    fn play_bgm_track(&self, track_name: &str) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Some(full_path) = BgmAssets::iter().find(|p| track_name_from_path(p) == track_name)
        else {
            return;
        };
        let Some(file) = BgmAssets::get(&full_path) else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        if let Ok(source) = rodio::Decoder::new(Cursor::new(file.data.into_owned())) {
            if let Some(old) = self.bgm_sink.borrow_mut().take() {
                old.stop();
            }
            sink.append(source.repeat_infinite());
            *self.bgm_sink.borrow_mut() = Some(sink);
        }
    }

    fn stop_bgm(&self) {
        if let Some(sink) = self.bgm_sink.borrow_mut().take() {
            sink.stop();
        }
    }

    /// タイプライターSEのループ再生を始める。既に同じ種類を再生中なら頭から鳴らし直さず、
    /// そのまま鳴らし続ける。別の種類が鳴っていれば、それを止めてから鳴らし直す。
    /// BGM用のSinkには触れない
    fn play_typewriter_loop(&self, kind: TypewriterSeKind) {
        if self.typewriter_looping.replace(Some(kind)) == Some(kind) {
            return;
        }
        if let Some(sink) = self.typewriter_sink.borrow_mut().take() {
            sink.stop();
        }
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        self.start_typewriter_sink(sink, kind);
    }

    /// sinkにループさせたタイプライターSEを入れ、止める時のために保持する
    fn start_typewriter_sink(&self, sink: Sink, kind: TypewriterSeKind) {
        if let Some(source) = typewriter_loop_source(kind) {
            sink.append(source);
            *self.typewriter_sink.borrow_mut() = Some(sink);
        }
    }

    /// タイプライターSEのループ再生を止める。BGM用のSinkには触れない
    fn stop_typewriter_loop(&self) {
        self.typewriter_looping.set(None);
        if let Some(sink) = self.typewriter_sink.borrow_mut().take() {
            sink.stop();
        }
    }

    #[cfg(test)]
    fn is_typewriter_loop_playing(&self) -> bool {
        self.typewriter_looping.get().is_some()
    }
}

thread_local! {
    // rodio::OutputStreamはSend/Syncでないため、シングルスレッドのTUIループ内で
    // thread_localとして保持する
    static PLAYER: RefCell<RodioPlayer> = RefCell::new(RodioPlayer::new());
}

pub fn play_se(se: SeKind) {
    PLAYER.with(|p| p.borrow().play_se(se));
}

pub fn play_tick() {
    PLAYER.with(|p| p.borrow().play_tick());
}

pub fn play_bgm_track(track_name: &str) {
    PLAYER.with(|p| p.borrow().play_bgm_track(track_name));
}

pub fn stop_bgm() {
    PLAYER.with(|p| p.borrow().stop_bgm());
}

/// タイプライターSEのループ再生を始める(BGMとは独立。既に同じ種類を再生中ならそのまま鳴らし続ける)
pub fn play_typewriter_loop(kind: TypewriterSeKind) {
    PLAYER.with(|p| p.borrow().play_typewriter_loop(kind));
}

/// タイプライターSEのループ再生を止める(BGMは止めない)
pub fn stop_typewriter_loop() {
    PLAYER.with(|p| p.borrow().stop_typewriter_loop());
}

/// タイプライターSEをループ再生中か(テストでの確認用)
#[cfg(test)]
pub fn is_typewriter_loop_playing() -> bool {
    PLAYER.with(|p| p.borrow().is_typewriter_loop_playing())
}

/// 現在ループ再生中のタイプライターSEの種類(テストでの確認用。鳴っていなければNone)
#[cfg(test)]
pub fn playing_typewriter_kind() -> Option<TypewriterSeKind> {
    PLAYER.with(|p| p.borrow().typewriter_looping.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SE_KINDS: [SeKind; 22] = [
        SeKind::Correct,
        SeKind::Incorrect,
        SeKind::Transition,
        SeKind::Confirm,
        SeKind::Star,
        SeKind::CountdownGo,
        SeKind::BeigomaSink,
        SeKind::BeigomaBump,
        SeKind::BeigomaFalloff,
        SeKind::BeigomaGoal,
        SeKind::QuickDrawReady,
        SeKind::QuickDrawShoot,
        SeKind::QuickDrawMiss,
        SeKind::GameSelect,
        SeKind::TtrSongSelect,
        SeKind::CountManiaPop,
        SeKind::CursorMove,
        SeKind::ColorStackClear,
        SeKind::ColorStackMiss,
        SeKind::LookAwayYahho,
        SeKind::LookAwayShout,
        SeKind::BeigomaSkid,
    ];

    #[test]
    fn asset_paths_are_distinct_per_se_kind() {
        // 合成音(Incorrect・Star)はファイルを持たない(None)ので、
        // 音源ファイルを持つ(Some)もの同士だけ重複が無いことを確認する
        let paths: Vec<&str> = ALL_SE_KINDS
            .iter()
            .filter_map(|se| se.asset_path())
            .collect();
        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                assert_ne!(paths[i], paths[j]);
            }
        }
    }

    #[test]
    fn every_se_asset_is_embedded() {
        for se in ALL_SE_KINDS {
            let Some(path) = se.asset_path() else {
                continue; // 合成音・複数候補からランダムに選ぶ音声はファイルを持たない
            };
            assert!(
                Assets::get(path).is_some(),
                "{se:?}のasset({path})が埋め込まれていること"
            );
        }
    }

    #[test]
    fn every_voice_candidate_is_embedded_and_distinct() {
        let mut all_paths = Vec::new();
        for se in [
            SeKind::LookAwayYahho,
            SeKind::LookAwayShout,
            SeKind::BeigomaSkid,
        ] {
            let candidates = se.voice_candidates().expect("候補一覧を持つこと");
            assert!(candidates.len() >= 2, "{se:?}: ランダムに選ぶ意味がある数の候補");
            for &path in candidates {
                assert!(
                    Assets::get(path).is_some(),
                    "{se:?}のasset({path})が埋め込まれていること"
                );
                all_paths.push(path);
            }
        }
        for i in 0..all_paths.len() {
            for j in (i + 1)..all_paths.len() {
                assert_ne!(all_paths[i], all_paths[j], "候補ファイルが重複していないこと");
            }
        }
    }

    // --- 生成音(tick音) ---

    #[test]
    fn tick_source_is_short_mono_beep() {
        let source = tick_source();
        assert_eq!(source.channels(), 1);
        let rate = source.sample_rate() as u128;
        // 長さはTICK_DURATIONぶんのサンプル数(端数の丸めで±1まで許容)
        let expected = rate * TICK_DURATION.as_micros() / 1_000_000;
        let count = source.count() as u128;
        assert!(
            count + 1 >= expected && count <= expected + 1,
            "サンプル数{count}(期待値{expected})"
        );
    }

    #[test]
    fn tick_source_is_quieter_than_full_volume_se() {
        // SEは音源ファイルそのままの音量(1.0倍)で鳴らすので、tick音はそれより小さくする
        let peak = tick_source().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak > 0.0, "無音ではないこと");
        assert!(peak < 1.0, "振幅{peak}が元の振幅(1.0)より小さいこと");
        assert!(peak <= TICK_VOLUME + 1e-6, "振幅{peak}が音量{TICK_VOLUME}以下");
    }

    #[test]
    fn play_tick_without_audio_device_does_not_panic() {
        // 音声デバイスが無い環境(handleがNone)では何もしない
        let player = RodioPlayer::without_device();
        player.play_tick();
    }

    // --- タイプライターSE(文字が流れている間のループ再生) ---

    /// 何か音が入っているSink(音声デバイスを使わないidleのSink)
    fn idle_sink_with_sound() -> (Sink, rodio::queue::SourcesQueueOutput<f32>) {
        let (sink, output) = Sink::new_idle();
        sink.append(tick_source());
        (sink, output)
    }

    #[test]
    fn typewriter_se_assets_are_embedded_and_decodable() {
        for kind in [TypewriterSeKind::Menu, TypewriterSeKind::Result] {
            let file = Assets::get(kind.asset_path())
                .unwrap_or_else(|| panic!("{}が埋め込まれていること", kind.asset_path()));
            assert!(
                rodio::Decoder::new(Cursor::new(file.data.into_owned())).is_ok(),
                "{:?}: mp3としてデコードできること",
                kind
            );
        }
    }

    #[test]
    fn typewriter_se_assets_are_distinct() {
        assert_ne!(
            TypewriterSeKind::Menu.asset_path(),
            TypewriterSeKind::Result.asset_path()
        );
    }

    #[test]
    fn typewriter_loop_source_repeats_the_se_endlessly() {
        // 1回分の長さを超えて取り出しても途切れない(repeat_infiniteでループしている)
        let file = Assets::get(TypewriterSeKind::Menu.asset_path()).unwrap();
        let one_pass = rodio::Decoder::new(Cursor::new(file.data.into_owned()))
            .unwrap()
            .count();
        assert!(one_pass > 0, "無音(空)ではないこと");
        let looped = typewriter_loop_source(TypewriterSeKind::Menu)
            .expect("ループ用の音源を作れること")
            .take(one_pass * 3)
            .count();
        assert_eq!(looped, one_pass * 3, "3周ぶん取り出せる");
    }

    #[test]
    fn typewriter_loop_is_off_initially() {
        let player = RodioPlayer::without_device();
        assert!(!player.is_typewriter_loop_playing());
    }

    #[test]
    fn play_and_stop_typewriter_loop_switch_its_state_without_audio_device() {
        let player = RodioPlayer::without_device();
        player.play_typewriter_loop(TypewriterSeKind::Menu);
        assert!(player.is_typewriter_loop_playing(), "再生中になる");
        player.play_typewriter_loop(TypewriterSeKind::Menu);
        assert!(
            player.is_typewriter_loop_playing(),
            "続けて呼んでも再生中のまま"
        );
        player.stop_typewriter_loop();
        assert!(!player.is_typewriter_loop_playing(), "止まる");
        player.stop_typewriter_loop();
        assert!(
            !player.is_typewriter_loop_playing(),
            "止まっている時に止めても何も起きない"
        );
    }

    #[test]
    fn play_typewriter_loop_with_a_different_kind_restarts_it() {
        let player = RodioPlayer::without_device();
        player.play_typewriter_loop(TypewriterSeKind::Menu);
        assert_eq!(
            player.typewriter_looping.get(),
            Some(TypewriterSeKind::Menu)
        );
        player.play_typewriter_loop(TypewriterSeKind::Result);
        assert_eq!(
            player.typewriter_looping.get(),
            Some(TypewriterSeKind::Result),
            "別の種類に切り替わる"
        );
    }

    #[test]
    fn start_typewriter_sink_appends_the_loop_and_keeps_the_sink() {
        let player = RodioPlayer::without_device();
        let (sink, _output) = Sink::new_idle();
        player.start_typewriter_sink(sink, TypewriterSeKind::Menu);
        let guard = player.typewriter_sink.borrow();
        let sink = guard.as_ref().expect("Sinkを保持している");
        assert!(!sink.empty(), "ループ用の音源が入っている");
    }

    #[test]
    fn stopping_typewriter_loop_stops_its_sink_but_not_the_bgm() {
        let player = RodioPlayer::without_device();
        let (bgm, _bgm_output) = idle_sink_with_sound();
        *player.bgm_sink.borrow_mut() = Some(bgm);
        player.play_typewriter_loop(TypewriterSeKind::Menu);
        let (typewriter, _typewriter_output) = Sink::new_idle();
        player.start_typewriter_sink(typewriter, TypewriterSeKind::Menu);

        player.stop_typewriter_loop();
        assert!(
            player.typewriter_sink.borrow().is_none(),
            "タイプライターSEのSinkは手放す"
        );
        let guard = player.bgm_sink.borrow();
        let bgm = guard.as_ref().expect("BGMのSinkはそのまま");
        assert!(!bgm.empty(), "BGMは止まらない");
    }

    #[test]
    fn stopping_or_switching_bgm_does_not_stop_the_typewriter_loop() {
        let player = RodioPlayer::without_device();
        player.play_typewriter_loop(TypewriterSeKind::Menu);
        let (typewriter, _output) = Sink::new_idle();
        player.start_typewriter_sink(typewriter, TypewriterSeKind::Menu);
        let (bgm, _bgm_output) = idle_sink_with_sound();
        *player.bgm_sink.borrow_mut() = Some(bgm);

        player.stop_bgm();
        player.play_bgm_track("Calculated_Play");
        assert!(player.is_typewriter_loop_playing());
        let guard = player.typewriter_sink.borrow();
        let sink = guard.as_ref().expect("タイプライターSEのSinkはそのまま");
        assert!(!sink.empty(), "タイプライターSEは止まらない");
    }

    #[test]
    fn public_typewriter_loop_functions_are_independent_of_bgm() {
        // 公開関数(スレッドごとのPLAYER)経由でも、BGMの停止・切り替えで状態が変わらない
        stop_typewriter_loop();
        assert!(!is_typewriter_loop_playing());
        play_typewriter_loop(TypewriterSeKind::Menu);
        assert!(is_typewriter_loop_playing());
        stop_bgm();
        play_bgm_track("Calculated_Play");
        assert!(is_typewriter_loop_playing(), "BGMを操作しても鳴り続ける");
        stop_typewriter_loop();
        assert!(!is_typewriter_loop_playing());
        stop_bgm();
    }

    // --- 生成音(不正解ブザー音) ---

    #[test]
    fn buzz_source_is_mono_at_expected_sample_rate() {
        let source = buzz_source();
        assert_eq!(source.channels(), 1);
        assert_eq!(source.sample_rate(), BUZZ_SAMPLE_RATE);
    }

    #[test]
    fn buzz_source_duration_matches_buzz_duration() {
        let source = buzz_source();
        let rate = source.sample_rate() as u128;
        let expected = rate * BUZZ_DURATION.as_micros() / 1_000_000;
        let count = source.count() as u128;
        assert!(
            count + 1 >= expected && count <= expected + 1,
            "サンプル数{count}(期待値{expected})"
        );
    }

    #[test]
    fn buzz_source_is_a_square_wave_not_a_smooth_sine() {
        // 矩形波は振幅がほぼ+振幅/-振幅の2値のみを取り、サイン波のような中間値がほとんど無い
        let samples: Vec<f32> = buzz_source().collect();
        assert!(!samples.is_empty());
        let peak = samples.iter().cloned().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak > 0.0, "無音ではないこと");
        let near_extreme = samples
            .iter()
            .filter(|&&s| s.abs() > peak * 0.9)
            .count();
        assert!(
            near_extreme as f64 > samples.len() as f64 * 0.9,
            "ほとんどのサンプルが振幅の頂点付近(矩形波)にあること: {near_extreme}/{}",
            samples.len()
        );
    }

    #[test]
    fn buzz_source_is_quieter_than_full_volume_se() {
        let peak = buzz_source().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak < 1.0, "振幅{peak}が元の振幅(1.0)より小さいこと");
        assert!(peak <= BUZZ_VOLUME + 1e-6, "振幅{peak}が音量{BUZZ_VOLUME}以下");
    }

    #[test]
    fn play_se_incorrect_without_audio_device_does_not_panic() {
        let player = RodioPlayer::without_device();
        player.play_se(SeKind::Incorrect);
    }

    // --- 生成音(「ふいっ」という音) ---

    #[test]
    fn whoosh_tones_sweep_downward() {
        for &(start_freq, end_freq, ..) in &WHOOSH_TONES {
            assert!(
                start_freq > end_freq,
                "「ふいっ」は高い音から低い音へ抜けるように下降する"
            );
        }
    }

    #[test]
    fn whoosh_source_is_mono_at_expected_sample_rate() {
        let source = whoosh_source();
        assert_eq!(source.channels(), 1);
        assert_eq!(source.sample_rate(), WHOOSH_SAMPLE_RATE);
    }

    #[test]
    fn whoosh_source_is_not_silent_but_quieter_than_full_volume_se() {
        let peak = whoosh_source().map(f32::abs).fold(0.0_f32, f32::max);
        assert!(peak > 0.0, "無音ではないこと");
        assert!(peak < 1.0, "振幅{peak}が元の振幅(1.0)より小さいこと");
    }

    #[test]
    fn whoosh_source_length_matches_the_last_tone_ending() {
        // 総サンプル数は、最後に終わる音(delay_ms + duration_ms)に一致する
        let expected_ms = WHOOSH_TONES
            .iter()
            .map(|&(_, _, duration_ms, _, _, delay_ms)| duration_ms + delay_ms)
            .max()
            .unwrap();
        let source = whoosh_source();
        let rate = source.sample_rate() as u128;
        let expected = rate * expected_ms as u128 / 1000;
        let count = source.count() as u128;
        assert!(
            count + 1 >= expected && count <= expected + 1,
            "サンプル数{count}(期待値{expected})"
        );
    }

    #[test]
    fn whoosh_source_is_short_enough_to_feel_instantaneous() {
        // 「ふいっ」は一瞬で消える音なので、200msを超えるような長い音にはしない
        let source = whoosh_source();
        let rate = source.sample_rate() as u128;
        let duration_ms = source.count() as u128 * 1000 / rate;
        assert!(duration_ms <= 200, "{duration_ms}msは長すぎる");
    }

    #[test]
    fn play_se_star_without_audio_device_does_not_panic() {
        let player = RodioPlayer::without_device();
        player.play_se(SeKind::Star);
    }

    #[test]
    fn track_name_from_path_strips_directory_and_extension() {
        assert_eq!(
            track_name_from_path("menu/Calculated_Play.mp3"),
            "Calculated_Play"
        );
    }

    #[test]
    fn track_name_from_path_without_extension_is_unchanged() {
        assert_eq!(track_name_from_path("no_extension"), "no_extension");
    }

    #[test]
    fn bgm_track_names_includes_calculated_play() {
        let names = bgm_track_names();
        assert!(
            names.iter().any(|n| n == "Calculated_Play"),
            "bgm/menu/Calculated_Play.mp3が一覧に含まれること: {names:?}"
        );
    }

    #[test]
    fn bgm_track_names_has_no_duplicates() {
        let names = bgm_track_names();
        let mut sorted = names.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len());
    }

    #[test]
    fn bgm_tracks_in_menu_includes_calculated_play_only() {
        let names = bgm_tracks_in(BgmCategory::Menu);
        assert!(names.iter().any(|n| n == "Calculated_Play"));
        assert!(!names.iter().any(|n| n == "Method_of_Thought"));
    }

    #[test]
    fn bgm_tracks_in_menu_has_two_tracks() {
        let names = bgm_tracks_in(BgmCategory::Menu);
        assert!(names.iter().any(|n| n == "Calculated_Play"));
        assert!(names.iter().any(|n| n == "Primary_Thruster"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn bgm_tracks_in_playing_includes_method_of_thought_only() {
        let names = bgm_tracks_in(BgmCategory::Playing);
        assert!(names.iter().any(|n| n == "Method_of_Thought"));
        assert!(!names.iter().any(|n| n == "Calculated_Play"));
    }

    #[test]
    fn random_bgm_track_returns_a_track_from_the_category() {
        let tracks = bgm_tracks_in(BgmCategory::Playing);
        let picked = random_bgm_track(BgmCategory::Playing);
        assert!(picked.is_some());
        assert!(tracks.contains(&picked.unwrap()));
    }

    #[test]
    fn bgm_tracks_in_playing_has_five_tracks() {
        let names = bgm_tracks_in(BgmCategory::Playing);
        assert!(names.iter().any(|n| n == "Method_of_Thought"));
        assert!(names.iter().any(|n| n == "Zenith_Pursuit"));
        assert!(names.iter().any(|n| n == "Apex_Calculation"));
        assert!(names.iter().any(|n| n == "Kinetic_Ascent"));
        assert!(names.iter().any(|n| n == "Beyond_the_Finish_Line"));
        assert_eq!(names.len(), 5);
    }

    #[test]
    fn bgm_tracks_in_result_has_new_personal_best_only() {
        let names = bgm_tracks_in(BgmCategory::Result);
        assert_eq!(names, vec!["New_Personal_Best".to_string()]);
    }

    #[test]
    fn bgm_tracks_in_result_failure_has_apex_pursuit_only() {
        let names = bgm_tracks_in(BgmCategory::ResultFailure);
        assert_eq!(names, vec!["Apex_Pursuit".to_string()]);
    }

    #[test]
    fn bgm_tracks_in_rhythm_has_all_rhythm_songs_only() {
        let names = bgm_tracks_in(BgmCategory::Rhythm);
        assert_eq!(
            names,
            vec![
                "Apex_Movement".to_string(),
                "Overclocked_Tempo".to_string(),
                "Redline_Response_Time".to_string(),
                "Top_of_the_Leaderboard".to_string()
            ]
        );
    }

    #[test]
    fn bgm_tracks_in_rhythm_splash_has_two_tracks() {
        let names = bgm_tracks_in(BgmCategory::RhythmSplash);
        assert!(names.iter().any(|n| n == "Overclocked_Tempo"));
        assert!(names.iter().any(|n| n == "Under_the_Floodlights"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn beigoma_splash_dir_prefix_is_beigoma_splash() {
        assert_eq!(BgmCategory::BeigomaSplash.dir_prefix(), "beigoma_splash/");
    }

    #[test]
    fn bgm_tracks_in_beigoma_splash_has_circuit_storm_only() {
        let names = bgm_tracks_in(BgmCategory::BeigomaSplash);
        assert_eq!(names, vec!["Circuit_Storm".to_string()]);
    }

    #[test]
    fn beigoma_splash_bgm_asset_is_embedded_and_decodable() {
        let file = BgmAssets::get("beigoma_splash/Circuit_Storm.mp3")
            .expect("bgm/beigoma_splash/Circuit_Storm.mp3が埋め込まれていること");
        assert!(
            rodio::Decoder::new(Cursor::new(file.data.into_owned())).is_ok(),
            "mp3としてデコードできること"
        );
    }

    #[test]
    fn beigoma_splash_bgm_is_not_mixed_into_playing_or_menu() {
        for category in [BgmCategory::Menu, BgmCategory::Playing] {
            let names = bgm_tracks_in(category);
            assert!(!names.iter().any(|n| n == "Circuit_Storm"), "{category:?}");
        }
    }

    #[test]
    fn random_bgm_track_for_beigoma_splash_picks_circuit_storm() {
        assert_eq!(
            random_bgm_track(BgmCategory::BeigomaSplash).as_deref(),
            Some("Circuit_Storm")
        );
    }

    #[test]
    fn beigoma_playing_dir_prefix_is_beigoma_playing() {
        assert_eq!(
            BgmCategory::BeigomaPlaying.dir_prefix(),
            "beigoma_playing/"
        );
    }

    #[test]
    fn bgm_tracks_in_beigoma_playing_has_two_tracks() {
        let names = bgm_tracks_in(BgmCategory::BeigomaPlaying);
        assert!(names.iter().any(|n| n == "Apex_Velocity"));
        assert!(names.iter().any(|n| n == "Midnight_Tarmac"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn beigoma_playing_bgm_assets_are_embedded_and_decodable() {
        for name in ["Apex_Velocity", "Midnight_Tarmac"] {
            let file = BgmAssets::get(&format!("beigoma_playing/{name}.mp3"))
                .unwrap_or_else(|| panic!("bgm/beigoma_playing/{name}.mp3が埋め込まれていること"));
            assert!(
                rodio::Decoder::new(Cursor::new(file.data.into_owned())).is_ok(),
                "{name}: mp3としてデコードできること"
            );
        }
    }

    #[test]
    fn beigoma_playing_bgm_is_not_mixed_into_playing_or_menu() {
        for category in [BgmCategory::Menu, BgmCategory::Playing] {
            let names = bgm_tracks_in(category);
            assert!(!names.iter().any(|n| n == "Apex_Velocity"), "{category:?}");
            assert!(
                !names.iter().any(|n| n == "Midnight_Tarmac"),
                "{category:?}"
            );
        }
    }

    #[test]
    fn random_bgm_track_for_beigoma_playing_eventually_picks_both_tracks() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            if let Some(name) = random_bgm_track(BgmCategory::BeigomaPlaying) {
                seen.insert(name);
            }
        }
        assert_eq!(seen.len(), 2, "100回試行して両曲が出現するはず: {seen:?}");
    }

    #[test]
    fn look_away_dir_prefix_is_look_away() {
        assert_eq!(BgmCategory::LookAway.dir_prefix(), "look_away/");
    }

    #[test]
    fn bgm_tracks_in_look_away_has_two_tracks() {
        let names = bgm_tracks_in(BgmCategory::LookAway);
        assert!(names.iter().any(|n| n == "Paper_Lanterns_at_Midnight"));
        assert!(names.iter().any(|n| n == "The_Last_Lantern_Glow"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn look_away_bgm_assets_are_embedded_and_decodable() {
        for name in ["Paper_Lanterns_at_Midnight", "The_Last_Lantern_Glow"] {
            let file = BgmAssets::get(&format!("look_away/{name}.mp3"))
                .unwrap_or_else(|| panic!("bgm/look_away/{name}.mp3が埋め込まれていること"));
            assert!(
                rodio::Decoder::new(Cursor::new(file.data.into_owned())).is_ok(),
                "{name}: mp3としてデコードできること"
            );
        }
    }

    #[test]
    fn look_away_bgm_is_not_mixed_into_playing_or_menu() {
        for category in [BgmCategory::Menu, BgmCategory::Playing] {
            let names = bgm_tracks_in(category);
            assert!(
                !names.iter().any(|n| n == "Paper_Lanterns_at_Midnight"),
                "{category:?}"
            );
            assert!(
                !names.iter().any(|n| n == "The_Last_Lantern_Glow"),
                "{category:?}"
            );
        }
    }

    #[test]
    fn random_bgm_track_for_look_away_eventually_picks_both_tracks() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            if let Some(name) = random_bgm_track(BgmCategory::LookAway) {
                seen.insert(name);
            }
        }
        assert_eq!(seen.len(), 2, "100回試行して両曲が出現するはず: {seen:?}");
    }

    #[test]
    fn rhythm_songs_are_not_mixed_into_playing_or_menu() {
        for category in [BgmCategory::Menu, BgmCategory::Playing] {
            let names = bgm_tracks_in(category);
            assert!(!names.iter().any(|n| n == "Top_of_the_Leaderboard"));
            assert!(!names.iter().any(|n| n == "Redline_Response_Time"));
            assert!(!names.iter().any(|n| n == "Apex_Movement"));
        }
    }

    #[test]
    fn random_bgm_track_eventually_picks_every_playing_track() {
        // 複数曲から選ばれることを、十分な試行回数で統計的に確認する
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            if let Some(name) = random_bgm_track(BgmCategory::Playing) {
                seen.insert(name);
            }
        }
        assert_eq!(seen.len(), 5, "100回試行して全曲が出現するはず: {seen:?}");
    }
}
