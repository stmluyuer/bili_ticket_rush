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
