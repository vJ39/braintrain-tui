use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

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
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let line = serde_json::to_string(result).map_err(io::Error::other)?;
    writeln!(file, "{line}")
}

pub fn load_all() -> io::Result<Vec<GameResult>> {
    let path = history_path()?;
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
