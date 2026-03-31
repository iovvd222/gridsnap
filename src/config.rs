use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// グリッド設定。モニターごとに上書き可能。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GridConfig {
    /// 横方向のグリッド数
    pub columns: u32,
    /// 縦方向のグリッド数
    pub rows: u32,
}

impl Default for GridConfig {
    fn default() -> Self {
        Self { columns: 20, rows: 12 }
    }
}

/// モニター別グリッド設定。キーはモニター名（GetMonitorInfoW の szDevice）。
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct MonitorGridConfig {
    pub columns: Option<u32>,
    pub rows: Option<u32>,
}

/// F0 自動配置ルール。
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AppRule {
    /// 対象モニター名（部分一致）。None = 全モニター共通。
    /// Windows: szDevice (例: "\\\\.\\DISPLAY1")
    /// macOS: CGDirectDisplayID の文字列表現
    #[serde(default)]
    pub monitor: Option<String>,
    /// ウィンドウクラス名（部分一致）
    pub class_name: Option<String>,
    /// 実行ファイル名（部分一致）
    pub exe_name: Option<String>,
    /// 配置先グリッドセル（0-indexed, 左上原点）
    pub col: u32,
    pub row: u32,
    /// 占有グリッド数
    pub col_span: u32,
    pub row_span: u32,
}

/// オーバーレイ表示設定（F4）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OverlayConfig {
    /// オーバーレイを有効にするか
    pub enabled: bool,
    /// グリッド線の色（ARGB）
    pub color_argb: u32,
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            color_argb: 0x40_00_80_FF, // 半透明の青
        }
    }
}

/// タイトルバー非表示設定（F5）
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct TitlebarConfig {
    /// タイトルバーを隠すウィンドウのクラス名リスト
    pub hide_for_classes: Vec<String>,
}

/// トップレベル設定
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub grid: GridConfig,
    /// モニター別グリッド上書き（キー: szDevice名）
    #[serde(default)]
    pub monitor_grids: HashMap<String, MonitorGridConfig>,
    /// F0 自動配置ルール
    #[serde(default)]
    pub app_rules: Vec<AppRule>,
    /// F0 自動配置の除外クラス名
    #[serde(default)]
    pub auto_place_exclude: Vec<String>,
    /// F4 オーバーレイ
    #[serde(default)]
    pub overlay: OverlayConfig,
    /// F5 タイトルバー非表示
    #[serde(default)]
    pub titlebar: TitlebarConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            grid: GridConfig::default(),
            monitor_grids: HashMap::new(),
            app_rules: Vec::new(),
            auto_place_exclude: vec![
                // デフォルト除外: タスクバー、通知領域、ポップアップ類
                "Shell_TrayWnd".into(),
                "NotifyIconOverflowWindow".into(),
                "tooltips_class32".into(),
            ],
            overlay: OverlayConfig::default(),
            titlebar: TitlebarConfig::default(),
        }
    }
}

impl Config {
    /// 設定ファイルを読み込む。存在しない場合はデフォルトを返す。
    pub fn load() -> Result<Self> {
        let path = Self::config_path();
        if !path.exists() {
            log::info!("Config not found at {:?}. Using defaults.", path);
            return Ok(Self::default());
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config: {:?}", path))?;
        let config: Self = toml::from_str(&text)
            .with_context(|| format!("Failed to parse config: {:?}", path))?;
        Ok(config)
    }

    /// 設定ファイルに書き出す。
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        let text = toml::to_string_pretty(self)
            .context("Failed to serialize config")?;
        fs::write(&path, &text)
            .with_context(|| format!("Failed to write config: {:?}", path))?;
        log::info!("Config saved to {:?}", path);
        Ok(())
    }

    /// 設定ファイルパス: exe と同じディレクトリの gridsnap.toml
    pub fn config_path() -> PathBuf {
        std::env::current_exe()
            .unwrap_or_default()
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("gridsnap.toml")
    }

    /// モニター名に対応するグリッド設定を返す。
    /// モニター固有設定があればそれで上書きし、なければ共通設定を返す。
    pub fn grid_for_monitor(&self, device_name: &str) -> GridConfig {
        if let Some(m) = self.monitor_grids.get(device_name) {
            GridConfig {
                columns: m.columns.unwrap_or(self.grid.columns),
                rows: m.rows.unwrap_or(self.grid.rows),
            }
        } else {
            self.grid.clone()
        }
    }

    /// app_rules にルールを upsert する（F0a）。
    /// 同一 (exe_name, monitor) ペアのルールがあれば上書きし、なければ追加する。
    pub fn upsert_app_rule(&mut self, rule: AppRule) {
        if let Some(existing) = self.app_rules.iter_mut().find(|r| {
            r.exe_name.is_some()
                && r.exe_name == rule.exe_name
                && r.monitor == rule.monitor
        }) {
            existing.col = rule.col;
            existing.row = rule.row;
            existing.col_span = rule.col_span;
            existing.row_span = rule.row_span;
            existing.class_name = rule.class_name.clone();
        } else {
            self.app_rules.push(rule);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ================================================================
    // 1. GridConfig デフォルト値
    // ================================================================

    #[test]
    fn default_grid_config() {
        let gc = GridConfig::default();
        assert_eq!(gc.columns, 20);
        assert_eq!(gc.rows, 12);
    }

    // ================================================================
    // 2. Config::grid_for_monitor — モニター別解決
    // ================================================================

    #[test]
    fn grid_for_monitor_default_when_no_override() {
        let config = Config::default();
        let gc = config.grid_for_monitor("\\\\.\\DISPLAY1");
        assert_eq!(gc.columns, 20);
        assert_eq!(gc.rows, 12);
    }

    #[test]
    fn grid_for_monitor_full_override() {
        let mut config = Config::default();
        config.monitor_grids.insert(
            "\\\\.\\DISPLAY2".into(),
            MonitorGridConfig { columns: Some(16), rows: Some(8) },
        );
        let gc = config.grid_for_monitor("\\\\.\\DISPLAY2");
        assert_eq!(gc.columns, 16);
        assert_eq!(gc.rows, 8);
    }

    #[test]
    fn grid_for_monitor_partial_override_columns_only() {
        let mut config = Config::default();
        config.monitor_grids.insert(
            "DISPLAY1".into(),
            MonitorGridConfig { columns: Some(32), rows: None },
        );
        let gc = config.grid_for_monitor("DISPLAY1");
        assert_eq!(gc.columns, 32);
        assert_eq!(gc.rows, 12);
    }

    #[test]
    fn grid_for_monitor_partial_override_rows_only() {
        let mut config = Config::default();
        config.monitor_grids.insert(
            "DISPLAY1".into(),
            MonitorGridConfig { columns: None, rows: Some(6) },
        );
        let gc = config.grid_for_monitor("DISPLAY1");
        assert_eq!(gc.columns, 20);
        assert_eq!(gc.rows, 6);
    }

    // ================================================================
    // 3. Config::upsert_app_rule
    // ================================================================

    #[test]
    fn upsert_app_rule_insert_new() {
        let mut config = Config::default();
        assert_eq!(config.app_rules.len(), 0);
        config.upsert_app_rule(AppRule {
            monitor: None,
            class_name: None,
            exe_name: Some("Code.exe".into()),
            col: 0, row: 0, col_span: 4, row_span: 4,
        });
        assert_eq!(config.app_rules.len(), 1);
        assert_eq!(config.app_rules[0].col_span, 4);
    }

    #[test]
    fn upsert_app_rule_overwrite_existing() {
        let mut config = Config::default();
        config.upsert_app_rule(AppRule {
            monitor: None,
            class_name: None,
            exe_name: Some("Code.exe".into()),
            col: 0, row: 0, col_span: 4, row_span: 4,
        });
        config.upsert_app_rule(AppRule {
            monitor: None,
            class_name: Some("Chrome_WidgetWin_1".into()),
            exe_name: Some("Code.exe".into()),
            col: 5, row: 1, col_span: 3, row_span: 2,
        });
        assert_eq!(config.app_rules.len(), 1);
        assert_eq!(config.app_rules[0].col, 5);
        assert_eq!(config.app_rules[0].col_span, 3);
    }

    #[test]
    fn upsert_app_rule_different_monitor_creates_new() {
        let mut config = Config::default();
        config.upsert_app_rule(AppRule {
            monitor: None,
            class_name: None,
            exe_name: Some("Code.exe".into()),
            col: 0, row: 0, col_span: 4, row_span: 4,
        });
        config.upsert_app_rule(AppRule {
            monitor: Some("DISPLAY2".into()),
            class_name: None,
            exe_name: Some("Code.exe".into()),
            col: 0, row: 0, col_span: 8, row_span: 4,
        });
        assert_eq!(config.app_rules.len(), 2);
    }

    #[test]
    fn upsert_app_rule_no_exe_name_always_appends() {
        let mut config = Config::default();
        config.upsert_app_rule(AppRule {
            monitor: None,
            class_name: Some("MyClass".into()),
            exe_name: None,
            col: 0, row: 0, col_span: 1, row_span: 1,
        });
        config.upsert_app_rule(AppRule {
            monitor: None,
            class_name: Some("MyClass".into()),
            exe_name: None,
            col: 1, row: 1, col_span: 2, row_span: 2,
        });
        assert_eq!(config.app_rules.len(), 2);
    }

    // ================================================================
    // 4. TOML シリアライズ/デシリアライズ
    // ================================================================

    #[test]
    fn config_roundtrip_toml() {
        let mut config = Config::default();
        config.grid.columns = 16;
        config.grid.rows = 8;
        config.upsert_app_rule(AppRule {
            monitor: Some("DISPLAY1".into()),
            class_name: None,
            exe_name: Some("firefox.exe".into()),
            col: 0, row: 0, col_span: 8, row_span: 4,
        });
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.grid.columns, 16);
        assert_eq!(parsed.grid.rows, 8);
        assert_eq!(parsed.app_rules.len(), 1);
        assert_eq!(parsed.app_rules[0].exe_name.as_deref(), Some("firefox.exe"));
    }

    #[test]
    fn config_deserialize_minimal_toml() {
        let toml_str = r#"
[grid]
columns = 8
rows = 4

[overlay]
enabled = true
color_argb = 1073774847

[titlebar]
hide_for_classes = []
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.grid.columns, 8);
        assert_eq!(config.app_rules.len(), 0);
        assert!(config.auto_place_exclude.is_empty());
    }

    // ================================================================
    // 5. OverlayConfig / TitlebarConfig デフォルト
    // ================================================================

    #[test]
    fn overlay_config_default() {
        let ov = OverlayConfig::default();
        assert!(ov.enabled);
        assert_eq!(ov.color_argb, 0x40_00_80_FF);
    }

    #[test]
    fn default_auto_place_exclude() {
        let config = Config::default();
        assert!(config.auto_place_exclude.contains(&"Shell_TrayWnd".to_string()));
        assert!(config.auto_place_exclude.contains(&"tooltips_class32".to_string()));
    }
}