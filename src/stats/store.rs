use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::game::GameResult;

fn history_path() -> io::Result<PathBuf> {
    let mut dir = dirs::data_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "data_dir not found"))?;
    dir.push("braintrain-tui");
    fs::create_dir_all(&dir)?;
    dir.push("history.jsonl");
    Ok(dir)
}

pub fn append_result(result: &GameResult) -> io::Result<()> {
    let path = history_path()?;
    append_result_to(&path, result)
}

pub fn load_all() -> io::Result<Vec<GameResult>> {
    let path = history_path()?;
    load_all_from(&path)
}

fn append_result_to(path: &Path, result: &GameResult) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(result).map_err(io::Error::other)?;
    writeln!(file, "{line}")
}

fn load_all_from(path: &Path) -> io::Result<Vec<GameResult>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(path)?;
    let results = content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Difficulty;
    use chrono::Utc;

    fn sample_result(game_id: &str, correct: u32) -> GameResult {
        GameResult {
            game_id: game_id.to_string(),
            difficulty: Difficulty::Beginner,
            correct,
            total: 10,
            avg_latency_ms: 123.4,
            played_at: Utc::now(),
            forced_game_over: false,
        }
    }

    /// テスト専用の一時ファイルパスを作る(テスト関数名で衝突を避ける)
    fn temp_path(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "braintrain-tui-test-{}-{}-{}.jsonl",
            std::process::id(),
            test_name,
            rand::random::<u32>()
        ))
    }

    #[test]
    fn load_all_from_missing_file_returns_empty() {
        let path = temp_path("missing");
        assert!(!path.exists());
        let results = load_all_from(&path).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn append_then_load_round_trips_a_single_record() {
        let path = temp_path("single");
        let result = sample_result("shape_rotate", 7);
        append_result_to(&path, &result).unwrap();

        let loaded = load_all_from(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].game_id, "shape_rotate");
        assert_eq!(loaded[0].correct, 7);
        assert_eq!(loaded[0].total, 10);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn append_multiple_times_accumulates_records_in_order() {
        let path = temp_path("multiple");
        append_result_to(&path, &sample_result("shape_rotate", 1)).unwrap();
        append_result_to(&path, &sample_result("mirror_match", 2)).unwrap();
        append_result_to(&path, &sample_result("reaction", 3)).unwrap();

        let loaded = load_all_from(&path).unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].game_id, "shape_rotate");
        assert_eq!(loaded[1].game_id, "mirror_match");
        assert_eq!(loaded[2].game_id, "reaction");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn load_all_from_skips_blank_lines() {
        let path = temp_path("blank_lines");
        let json = serde_json::to_string(&sample_result("mental_calc", 5)).unwrap();
        fs::write(&path, format!("\n{json}\n\n")).unwrap();

        let loaded = load_all_from(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].game_id, "mental_calc");

        let _ = fs::remove_file(&path);
    }
}
