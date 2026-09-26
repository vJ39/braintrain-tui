use std::cell::RefCell;
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BgmCategory {
    Menu,
    Playing,
    /// ゲーム終了後のリザルト画面
    Result,
    /// リズムゲームの楽曲。再生は譜面と対応する曲をトラック名で直接指定するため
    /// ランダム選曲には使わず、曲データとassetsの対応確認(テスト)で参照する
    #[cfg_attr(not(test), allow(dead_code))]
    Rhythm,
    /// TTR専用スプラッシュ画面〜曲選択画面の間に流す専用BGM
    RhythmSplash,
}

impl BgmCategory {
    fn dir_prefix(self) -> &'static str {
        match self {
            BgmCategory::Menu => "menu/",
            BgmCategory::Playing => "playing/",
            BgmCategory::Result => "result/",
            BgmCategory::Rhythm => "rhythm/",
            BgmCategory::RhythmSplash => "rhythm_splash/",
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

impl SeKind {
    fn asset_path(self) -> &'static str {
        match self {
            SeKind::Correct => "se_correct.wav",
            SeKind::Incorrect => "se_incorrect.wav",
            SeKind::Transition => "se_transition.wav",
            SeKind::Confirm => "se_confirm.wav",
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

/// 実際にrodioで音声デバイスへ再生するプレイヤー。
/// 音声デバイスが無い/取得できない環境では初期化時にNoneとなり、以後は何もしない。
pub struct RodioPlayer {
    handle: Option<(OutputStream, OutputStreamHandle)>,
    /// 現在ループ再生しているBGMのSink。次の曲に切り替える/停止するとき使う
    bgm_sink: RefCell<Option<Sink>>,
}

impl RodioPlayer {
    pub fn new() -> Self {
        let handle = OutputStream::try_default().ok();
        Self {
            handle,
            bgm_sink: RefCell::new(None),
        }
    }

    fn play_se(&self, se: SeKind) {
        let Some((_, stream_handle)) = &self.handle else {
            return;
        };
        let Ok(sink) = Sink::try_new(stream_handle) else {
            return;
        };
        // 不正解音は音源ファイルを使わず、耳障りな矩形波のブザー音を生成して鳴らす
        if se == SeKind::Incorrect {
            sink.append(buzz_source());
            sink.detach();
            return;
        }
        let Some(file) = Assets::get(se.asset_path()) else {
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

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_SE_KINDS: [SeKind; 4] = [
        SeKind::Correct,
        SeKind::Incorrect,
        SeKind::Transition,
        SeKind::Confirm,
    ];

    #[test]
    fn asset_paths_are_distinct_per_se_kind() {
        let paths = ALL_SE_KINDS.map(SeKind::asset_path);
        for i in 0..paths.len() {
            for j in (i + 1)..paths.len() {
                assert_ne!(paths[i], paths[j]);
            }
        }
    }

    #[test]
    fn every_se_asset_is_embedded() {
        for se in ALL_SE_KINDS {
            assert!(
                Assets::get(se.asset_path()).is_some(),
                "{:?}のasset({})が埋め込まれていること",
                se,
                se.asset_path()
            );
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
        let player = RodioPlayer {
            handle: None,
            bgm_sink: RefCell::new(None),
        };
        player.play_tick();
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
        let player = RodioPlayer {
            handle: None,
            bgm_sink: RefCell::new(None),
        };
        player.play_se(SeKind::Incorrect);
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
    fn bgm_tracks_in_playing_has_seven_tracks() {
        let names = bgm_tracks_in(BgmCategory::Playing);
        assert!(names.iter().any(|n| n == "Method_of_Thought"));
        assert!(names.iter().any(|n| n == "The_Quiet_Calculation"));
        assert!(names.iter().any(|n| n == "Zenith_Pursuit"));
        assert!(names.iter().any(|n| n == "Apex_Calculation"));
        assert!(names.iter().any(|n| n == "Kinetic_Ascent"));
        assert!(names.iter().any(|n| n == "Ten_Thousand_Strikes"));
        assert!(names.iter().any(|n| n == "Beyond_the_Finish_Line"));
        assert_eq!(names.len(), 7);
    }

    #[test]
    fn bgm_tracks_in_result_has_new_personal_best_only() {
        let names = bgm_tracks_in(BgmCategory::Result);
        assert_eq!(names, vec!["New_Personal_Best".to_string()]);
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
        assert_eq!(seen.len(), 7, "100回試行して全曲が出現するはず: {seen:?}");
    }
}
