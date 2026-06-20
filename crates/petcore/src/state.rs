//! Runtime state (pet position, pins, last session, last-pet time).
//! Persisted as JSON under ~/.config/cc-pet/state.json, with a one-time
//! migration read from the old ~/.config/claude-pet/state.json.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub pet_x: Option<i32>,
    pub pet_y: Option<i32>,
    pub last_session: Option<String>,
    pub pinned: Vec<String>,
    pub last_pet_time: Option<f64>,
}

fn config_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "cc-pet")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".config/cc-pet"))
}

pub fn state_path() -> PathBuf {
    config_dir().join("state.json")
}

fn legacy_path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|b| b.home_dir().join(".config/claude-pet/state.json"))
}

impl State {
    pub fn load() -> State {
        let p = state_path();
        if let Ok(data) = fs::read_to_string(&p) {
            if let Ok(s) = serde_json::from_str::<State>(&data) {
                return s;
            }
        }
        // Migrate from the old Python app's state file if present.
        if let Some(lp) = legacy_path() {
            if let Ok(data) = fs::read_to_string(&lp) {
                if let Ok(s) = serde_json::from_str::<State>(&data) {
                    s.save();
                    return s;
                }
            }
        }
        State::default()
    }

    pub fn save(&self) {
        let p = state_path();
        if let Some(parent) = p.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(data) = serde_json::to_string_pretty(self) {
            let _ = fs::write(&p, data);
        }
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.pinned.iter().any(|p| p == path)
    }

    pub fn toggle_pin(&mut self, path: &str) {
        if let Some(i) = self.pinned.iter().position(|p| p == path) {
            self.pinned.remove(i);
        } else {
            self.pinned.push(path.to_string());
            self.pinned.sort();
        }
    }
}
