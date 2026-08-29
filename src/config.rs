use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

use crate::model::ModelKind;

pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AppConfig {
    pub version: u32,
    pub microphone: Option<String>,
    pub language: String,
    pub wake_phrase: String,
    pub model: ModelKind,
    pub silence_ms: u32,
    pub max_message_seconds: u32,
    pub typing_delay_ms: u32,
    pub focus_timeout_seconds: u32,
    pub open_key: String,
    pub submit_key: String,
    pub launch_at_login: bool,
    pub min_confidence: f32,
    pub vad_threshold: f32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: CURRENT_CONFIG_VERSION,
            microphone: None,
            language: "en".to_owned(),
            wake_phrase: "game chat".to_owned(),
            model: ModelKind::TinyQ5_1,
            silence_ms: 700,
            max_message_seconds: 15,
            typing_delay_ms: 15,
            focus_timeout_seconds: 10,
            open_key: "Enter".to_owned(),
            submit_key: "Enter".to_owned(),
            launch_at_login: false,
            min_confidence: 0.35,
            vad_threshold: 0.012,
        }
    }
}

impl AppConfig {
    pub fn validate(&self) -> Result<()> {
        if self.version != CURRENT_CONFIG_VERSION {
            bail!("unsupported settings version {}", self.version);
        }
        if self.language.trim().is_empty() {
            bail!("recognition language cannot be empty");
        }
        if self.wake_phrase.trim().is_empty() {
            bail!("wake phrase cannot be empty");
        }
        if !(200..=3_000).contains(&self.silence_ms) {
            bail!("end-of-message silence must be between 200 and 3000 ms");
        }
        if !(2..=60).contains(&self.max_message_seconds) {
            bail!("maximum message duration must be between 2 and 60 seconds");
        }
        if self.typing_delay_ms > 250 {
            bail!("typing delay must be at most 250 ms");
        }
        if self.focus_timeout_seconds > 120 {
            bail!("focus timeout must be at most 120 seconds");
        }
        if !(0.0..=1.0).contains(&self.min_confidence) {
            bail!("minimum confidence must be between 0 and 1");
        }
        if !(0.001..=0.2).contains(&self.vad_threshold) {
            bail!("voice threshold must be between 0.001 and 0.2");
        }
        crate::platform::input::KeySpec::parse(&self.open_key).context("invalid open-chat key")?;
        crate::platform::input::KeySpec::parse(&self.submit_key).context("invalid submit key")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ConfigStore {
    root: PathBuf,
}

impl ConfigStore {
    pub fn discover() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "GameTranscribe", "GameTranscribe")
            .context("Windows did not provide a local application-data directory")?;
        Ok(Self {
            root: dirs.data_local_dir().to_path_buf(),
        })
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn models_dir(&self) -> PathBuf {
        self.root.join("models")
    }

    pub fn load(&self) -> Result<AppConfig> {
        let path = self.root.join("config.json");
        if !path.exists() {
            return Ok(AppConfig::default());
        }
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let config: AppConfig = serde_json::from_slice(&bytes)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        config.validate()?;
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create {}", self.root.display()))?;
        let target = self.root.join("config.json");
        let temporary = self.root.join("config.json.new");
        let data = serde_json::to_vec_pretty(config)?;
        fs::write(&temporary, data)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        crate::platform::files::replace_file(&temporary, &target)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        let thread_name = std::thread::current()
            .name()
            .unwrap_or("thread")
            .replace([':', '\\', '/'], "_");
        std::env::temp_dir().join(format!(
            "game-transcribe-{name}-{}-{}",
            std::process::id(),
            thread_name
        ))
    }

    #[test]
    fn defaults_are_valid() {
        AppConfig::default().validate().unwrap();
    }

    #[test]
    fn save_and_load_round_trip() {
        let root = test_root("config-round-trip");
        let _ = fs::remove_dir_all(&root);
        let store = ConfigStore::at(&root);
        let expected = AppConfig {
            wake_phrase: "team radio".to_owned(),
            ..AppConfig::default()
        };
        store.save(&expected).unwrap();
        assert_eq!(store.load().unwrap(), expected);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_dangerously_long_focus_timeout() {
        let config = AppConfig {
            focus_timeout_seconds: 121,
            ..AppConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn missing_fields_receive_current_defaults() {
        let config: AppConfig = serde_json::from_str(r#"{"wake_phrase":"radio"}"#).unwrap();
        assert_eq!(config.version, CURRENT_CONFIG_VERSION);
        assert_eq!(config.wake_phrase, "radio");
        assert_eq!(config.model, ModelKind::TinyQ5_1);
        config.validate().unwrap();
    }
}
