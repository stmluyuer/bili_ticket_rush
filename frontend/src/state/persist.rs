use std::collections::HashMap;
use common::{push::PushConfig, utility::CustomConfig, utils::{self, Config}};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode { System, Light, Dark }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UiConfig { pub theme: ThemeMode, pub scale: f32 }

impl Default for UiConfig {
    fn default() -> Self { Self { theme: ThemeMode::System, scale: 1.0 } }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Contact { pub name: String, pub tel: String }

#[derive(Clone, Debug)]
pub enum SaveState { Idle, Saved, Saving, Failed(String) }

pub fn load_ui(config: &Config) -> UiConfig {
    serde_json::from_value(config["ui_config"].clone()).unwrap_or_default()
}

pub fn load_skip_words(config: &Config) -> Vec<String> {
    serde_json::from_value(config["skip_words"].clone()).unwrap_or_default()
}

pub fn load_contacts(config: &Config) -> HashMap<String, Contact> {
    serde_json::from_value(config["contacts"].clone()).unwrap_or_default()
}

pub fn load_project_history(config: &Config) -> Vec<String> {
    let mut history: Vec<String> = serde_json::from_value(config["project_history"].clone()).unwrap_or_default();
    for id in ["1006859", "1007551"] {
        if !history.iter().any(|entry| entry == id) {
            history.push(id.to_owned());
        }
    }
    history
}

pub fn save_project_history(config: &mut Config, history: &[String]) -> Result<(), String> {
    config["project_history"] = serde_json::to_value(history).map_err(|e| e.to_string())?;
    config.save_config().map_err(|e| e.to_string())
}

pub fn save_settings(config: &mut Config, push: &PushConfig, custom: &CustomConfig, ui: &UiConfig) -> Result<(), String> {
    config["ui_config"] = serde_json::to_value(ui).map_err(|e| e.to_string())?;
    utils::save_config(config, Some(push), Some(custom), None).map(|_| ())
}

pub fn save_skip_words(config: &mut Config, words: &[String]) -> Result<(), String> {
    config["skip_words"] = serde_json::to_value(words).map_err(|e| e.to_string())?;
    config.save_config().map_err(|e| e.to_string())
}

pub fn save_contacts(config: &mut Config, contacts: &HashMap<String, Contact>) -> Result<(), String> {
    config["contacts"] = serde_json::to_value(contacts).map_err(|e| e.to_string())?;
    config.save_config().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, fs, process::Command};

    #[test]
    fn purchase_settings_round_trip() {
        const CHILD_ENV: &str = "BILI_TICKET_RUSH_SETTINGS_TEST_CHILD";
        if env::var_os(CHILD_ENV).is_some() {
            let mut config = Config::new();
            let mut custom = CustomConfig::new();
            custom.stock_check = true;
            custom.sale_delay_ms = 275;
            custom.stock_interval_ms = 3200;
            custom.create_interval_ms = 825;
            let ui = UiConfig { theme: ThemeMode::Dark, scale: 1.1 };

            save_settings(&mut config, &PushConfig::new(), &custom, &ui).unwrap();
            let loaded = Config::load_config().unwrap();
            let custom: CustomConfig = serde_json::from_value(loaded["custom_config"].clone()).unwrap();
            assert!(custom.stock_check);
            assert_eq!(custom.sale_delay_ms, 275);
            assert_eq!(custom.stock_interval_ms, 3200);
            assert_eq!(custom.create_interval_ms, 825);
            assert_eq!(load_ui(&loaded).theme, ThemeMode::Dark);
            return;
        }

        // Config 使用相对路径，独立进程可隔离文件且不改变其它测试的工作目录。
        let dir = env::temp_dir().join(format!("bili-ticket-settings-{:016x}", rand::random::<u64>()));
        fs::create_dir(&dir).unwrap();
        let result = Command::new(env::current_exe().unwrap())
            .current_dir(&dir)
            .env(CHILD_ENV, "1")
            .args(["--exact", "state::persist::tests::purchase_settings_round_trip"])
            .output();
        fs::remove_dir_all(dir).unwrap();
        let output = result.unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.contains("1 passed"), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
    }
}
