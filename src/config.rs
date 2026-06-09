use serde::{Deserialize, Serialize};
use std::path::Path;

/// Configuration for process discovery behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryConfig {
    /// Interval in seconds between process tree scans.
    pub scan_interval_secs: f64,
}

/// Configuration for energy collection behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CollectionConfig {
    /// Collection rate in Hz.
    pub rate_hz: f64,
    /// Maximum trace retention in seconds before rotation.
    pub trace_retention_secs: u64,
    /// Interval in seconds between trace recorder flushes.
    pub trace_flush_interval_secs: f64,
}

/// Configuration for the interactive terminal UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TuiConfig {
    /// Lower collection rate used by default for monitor-all TUI mode.
    pub monitor_all_rate_hz: f64,
    /// Lower process scan cadence used by default for monitor-all TUI mode.
    pub monitor_all_scan_interval_secs: f64,
    /// TUI render/input polling interval in milliseconds.
    pub render_interval_millis: u64,
}

/// Configuration for user-facing measurement units.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MeasurementUnitsConfig {
    /// Unit used when reporting energy at output boundaries.
    pub energy: String,
    /// Unit used when reporting power at output boundaries.
    pub power: String,
}

/// Top-level EMT configuration with layered resolution.
///
/// Resolution precedence (highest wins):
/// 1. CLI arguments (applied programmatically after loading)
/// 2. Project-local: `./emt.yaml`
/// 3. User-level: `~/.config/emt/config.yaml`
///
/// Missing files are silently skipped. Missing keys use compiled defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct EmtConfig {
    pub discovery: DiscoveryConfig,
    pub collection: CollectionConfig,
    pub tui: TuiConfig,
    pub measurement_units: MeasurementUnitsConfig,
}

/// Errors that can occur while loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("Failed to parse YAML: {0}")]
    Yaml(String),
    #[error("Invalid config value: {0}")]
    Invalid(String),
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            scan_interval_secs: 2.0,
        }
    }
}

impl Default for CollectionConfig {
    fn default() -> Self {
        Self {
            rate_hz: 10.0,
            trace_retention_secs: 3600,
            trace_flush_interval_secs: 5.0,
        }
    }
}

impl Default for TuiConfig {
    fn default() -> Self {
        Self {
            monitor_all_rate_hz: 0.1,
            monitor_all_scan_interval_secs: 30.0,
            render_interval_millis: 2000,
        }
    }
}

impl Default for MeasurementUnitsConfig {
    fn default() -> Self {
        Self {
            energy: "Joules".to_string(),
            power: "Watts".to_string(),
        }
    }
}

impl MeasurementUnitsConfig {
    /// Convert canonical Joules to the configured energy unit.
    pub fn convert_energy_from_joules(&self, joules: f64) -> f64 {
        joules / energy_unit_factor_to_joules(&self.energy).unwrap_or(1.0)
    }

    /// Convert canonical Watts to the configured power unit.
    pub fn convert_power_from_watts(&self, watts: f64) -> f64 {
        watts / power_unit_factor_to_watts(&self.power).unwrap_or(1.0)
    }

    /// Convert a value in the configured energy unit back to Joules.
    pub fn convert_energy_to_joules(&self, value: f64) -> f64 {
        value * energy_unit_factor_to_joules(&self.energy).unwrap_or(1.0)
    }
}

fn energy_unit_factor_to_joules(unit: &str) -> Option<f64> {
    match unit {
        "Joules" => Some(1.0),
        "kJ" => Some(1_000.0),
        "\u{03bc}J" | "uJ" => Some(1e-6),
        "mJ" => Some(1e-3),
        "Wh" => Some(3_600.0),
        "kWh" => Some(3_600_000.0),
        _ => None,
    }
}

fn power_unit_factor_to_watts(unit: &str) -> Option<f64> {
    match unit {
        "Watts" => Some(1.0),
        "kW" => Some(1_000.0),
        "mW" => Some(1e-3),
        _ => None,
    }
}

impl EmtConfig {
    /// Load config with layered resolution.
    ///
    /// Reads user-level config first (`~/.config/emt/config.yaml`), then overlays
    /// project-local config (`./emt.yaml`) on top. Missing files are silently
    /// skipped and defaults are returned.
    pub fn load() -> Self {
        let mut base = serde_yml::to_value(EmtConfig::default())
            .unwrap_or(serde_yml::Value::Mapping(serde_yml::Mapping::new()));

        // Layer 1: user-level config
        if let Some(user_path) = Self::user_config_path() {
            if let Ok(content) = std::fs::read_to_string(&user_path) {
                if let Ok(user_value) = serde_yml::from_str::<serde_yml::Value>(&content) {
                    base = merge_yaml(base, user_value);
                }
            }
        }

        // Layer 2: project-local config (highest file priority)
        let local_path = Path::new("./emt.yaml");
        if let Ok(content) = std::fs::read_to_string(local_path) {
            if let Ok(local_value) = serde_yml::from_str::<serde_yml::Value>(&content) {
                base = merge_yaml(base, local_value);
            }
        }

        serde_yml::from_value::<Self>(base)
            .map(Self::validated_or_default)
            .unwrap_or_default()
    }

    /// Load configuration from a specific YAML file path.
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Self =
            serde_yml::from_str(&content).map_err(|e| ConfigError::Yaml(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Validate config values that are later used to build durations or rates.
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_positive_finite(
            "discovery.scan_interval_secs",
            self.discovery.scan_interval_secs,
        )?;
        validate_positive_finite("collection.rate_hz", self.collection.rate_hz)?;
        validate_positive_finite(
            "collection.trace_flush_interval_secs",
            self.collection.trace_flush_interval_secs,
        )?;
        validate_positive_finite("tui.monitor_all_rate_hz", self.tui.monitor_all_rate_hz)?;
        validate_positive_finite(
            "tui.monitor_all_scan_interval_secs",
            self.tui.monitor_all_scan_interval_secs,
        )?;
        if self.tui.render_interval_millis == 0 {
            return Err(ConfigError::Invalid(
                "tui.render_interval_millis must be greater than 0".to_string(),
            ));
        }
        if self.collection.trace_retention_secs == 0 {
            return Err(ConfigError::Invalid(
                "collection.trace_retention_secs must be greater than 0".to_string(),
            ));
        }
        Ok(())
    }

    fn validated_or_default(config: Self) -> Self {
        if config.validate().is_ok() {
            config
        } else {
            Self::default()
        }
    }

    /// Returns the user-level config path: `~/.config/emt/config.yaml`
    fn user_config_path() -> Option<std::path::PathBuf> {
        dirs::config_dir().map(|dir| dir.join("emt").join("config.yaml"))
    }
}

fn validate_positive_finite(name: &str, value: f64) -> Result<(), ConfigError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(ConfigError::Invalid(format!(
            "{name} must be a finite value greater than 0"
        )))
    }
}

/// Deep-merge two YAML values. Mappings are merged recursively;
/// scalars and sequences in the overlay replace the base entirely.
fn merge_yaml(base: serde_yml::Value, overlay: serde_yml::Value) -> serde_yml::Value {
    match (base, overlay) {
        (serde_yml::Value::Mapping(mut base_map), serde_yml::Value::Mapping(overlay_map)) => {
            for (key, value) in overlay_map {
                let merged = if let Some(base_value) = base_map.remove(&key) {
                    merge_yaml(base_value, value)
                } else {
                    value
                };
                base_map.insert(key, merged);
            }
            serde_yml::Value::Mapping(base_map)
        }
        (_, overlay) => overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn defaults_are_sensible() {
        let config = EmtConfig::default();
        assert_eq!(config.discovery.scan_interval_secs, 2.0);
        assert_eq!(config.collection.rate_hz, 10.0);
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
        assert_eq!(config.tui.monitor_all_rate_hz, 0.1);
        assert_eq!(config.tui.monitor_all_scan_interval_secs, 30.0);
        assert_eq!(config.tui.render_interval_millis, 2000);
        assert_eq!(config.measurement_units.energy, "Joules");
        assert_eq!(config.measurement_units.power, "Watts");
    }

    #[test]
    fn partial_yaml_fills_defaults() {
        let yaml = "collection:\n  rate_hz: 20.0\n";
        let config: EmtConfig = serde_yml::from_str(yaml).unwrap();
        assert_eq!(config.collection.rate_hz, 20.0);
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
        assert_eq!(config.discovery.scan_interval_secs, 2.0);
        assert_eq!(config.tui.monitor_all_rate_hz, 0.1);
        assert_eq!(config.measurement_units.energy, "Joules");
    }

    #[test]
    fn empty_yaml_returns_defaults() {
        let yaml = "{}";
        let config: EmtConfig = serde_yml::from_str(yaml).unwrap();
        assert_eq!(config.collection.rate_hz, 10.0);
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
        assert_eq!(config.discovery.scan_interval_secs, 2.0);
        assert_eq!(config.tui.render_interval_millis, 2000);
    }

    #[test]
    fn from_file_reads_valid_yaml() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test_config.yaml");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(
            file,
            "discovery:\n  scan_interval_secs: 5.0\ncollection:\n  rate_hz: 25.0"
        )
        .unwrap();

        let config = EmtConfig::from_file(&file_path).unwrap();
        assert_eq!(config.discovery.scan_interval_secs, 5.0);
        assert_eq!(config.collection.rate_hz, 25.0);
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
    }

    #[test]
    fn from_file_rejects_non_positive_rate() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("bad_rate.yaml");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(file, "collection:\n  rate_hz: 0.0").unwrap();

        let result = EmtConfig::from_file(&file_path);

        assert!(matches!(result, Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn validate_rejects_invalid_runtime_durations() {
        let mut config = EmtConfig::default();
        config.discovery.scan_interval_secs = f64::NAN;
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));

        let mut config = EmtConfig::default();
        config.collection.trace_flush_interval_secs = -1.0;
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));

        let mut config = EmtConfig::default();
        config.collection.trace_retention_secs = 0;
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));

        let mut config = EmtConfig::default();
        config.tui.render_interval_millis = 0;
        assert!(matches!(config.validate(), Err(ConfigError::Invalid(_))));
    }

    #[test]
    fn from_file_returns_error_for_missing_file() {
        let result = EmtConfig::from_file(Path::new("/nonexistent/path/config.yaml"));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::Io(_)));
    }

    #[test]
    fn from_file_returns_error_for_invalid_yaml() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("bad.yaml");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(file, "collection:\n  rate_hz: [invalid").unwrap();

        let result = EmtConfig::from_file(&file_path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ConfigError::Yaml(_)));
    }

    #[test]
    fn merge_yaml_deep_merges_mappings() {
        let base_yaml = "discovery:\n  scan_interval_secs: 2.0\ncollection:\n  rate_hz: 10.0\n  trace_retention_secs: 3600\n";
        let overlay_yaml = "collection:\n  rate_hz: 50.0\n";

        let base: serde_yml::Value = serde_yml::from_str(base_yaml).unwrap();
        let overlay: serde_yml::Value = serde_yml::from_str(overlay_yaml).unwrap();

        let merged = merge_yaml(base, overlay);
        let config: EmtConfig = serde_yml::from_value(merged).unwrap();

        assert_eq!(config.collection.rate_hz, 50.0);
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
        assert_eq!(config.discovery.scan_interval_secs, 2.0);
    }

    #[test]
    fn local_overrides_user_config() {
        // Simulate the merge sequence: defaults -> user -> local
        let user_yaml = "discovery:\n  scan_interval_secs: 5.0\ncollection:\n  rate_hz: 20.0\n";
        let local_yaml = "collection:\n  rate_hz: 100.0\n";

        let defaults = serde_yml::to_value(EmtConfig::default()).unwrap();
        let user: serde_yml::Value = serde_yml::from_str(user_yaml).unwrap();
        let local: serde_yml::Value = serde_yml::from_str(local_yaml).unwrap();

        let merged = merge_yaml(merge_yaml(defaults, user), local);
        let config: EmtConfig = serde_yml::from_value(merged).unwrap();

        // local overrides user's rate_hz
        assert_eq!(config.collection.rate_hz, 100.0);
        // user's scan_interval_secs is preserved (local didn't touch it)
        assert_eq!(config.discovery.scan_interval_secs, 5.0);
        // defaults fill trace_retention_secs (neither user nor local set it)
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
    }

    #[test]
    fn missing_files_return_defaults() {
        // load() with no files on disk should return defaults without panicking
        let config = EmtConfig::load();
        assert_eq!(config.collection.rate_hz, 10.0);
        assert_eq!(config.collection.trace_retention_secs, 3600);
        assert_eq!(config.collection.trace_flush_interval_secs, 5.0);
        assert_eq!(config.discovery.scan_interval_secs, 2.0);
    }

    #[test]
    fn user_config_path_is_under_config_dir() {
        if let Some(path) = EmtConfig::user_config_path() {
            assert!(path.ends_with("emt/config.yaml"));
        }
    }

    #[test]
    fn measurement_units_convert_from_canonical_values() {
        let units = MeasurementUnitsConfig {
            energy: "kWh".to_string(),
            power: "mW".to_string(),
        };

        assert!((units.convert_energy_from_joules(3_600_000.0) - 1.0).abs() < 1e-9);
        assert!((units.convert_power_from_watts(2.5) - 2_500.0).abs() < 1e-9);
    }

    #[test]
    fn measurement_units_support_python_microjoule_spelling() {
        let units = MeasurementUnitsConfig {
            energy: "\u{03bc}J".to_string(),
            power: "Watts".to_string(),
        };

        assert!((units.convert_energy_from_joules(1.0) - 1_000_000.0).abs() < 1e-9);
    }
}
