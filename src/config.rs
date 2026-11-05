use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The only configuration file boxxy reads or writes.
pub const CONFIG_RELATIVE: &str = ".config/boxxycli/boxxy.toml";

pub const DEFAULT_CONFIG: &str = "\
[frame]
thickness = 1
radius = \"rounded\"
color = \"#89DCEB\"

[brand]
text = \"BOXXY\"
";

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_BRAND_CHARS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Thickness {
    One,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Radius {
    Square,
    Rounded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub thickness: Thickness,
    pub radius: Radius,
    pub color: Rgb,
    pub brand: String,
}

#[derive(Debug)]
pub enum ConfigError {
    Missing,
    Exists(PathBuf),
    Detail(String),
}

impl ConfigError {
    fn detail(message: impl Into<String>) -> Self {
        Self::Detail(message.into())
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "No configuration found.\nRun 'boxxy --generate-config' to create one."
            ),
            Self::Exists(path) => write!(
                f,
                "Error:\nConfiguration already exists: {}\nRun 'boxxy --generate-config --force' to overwrite it.",
                path.display()
            ),
            Self::Detail(message) => write!(f, "Error:\n{message}"),
        }
    }
}

impl std::error::Error for ConfigError {}

pub fn config_path_in(home: &Path) -> PathBuf {
    home.join(CONFIG_RELATIVE)
}

pub fn home_dir() -> Result<PathBuf, ConfigError> {
    let home = std::env::var_os("HOME").filter(|value| !value.is_empty());
    let Some(home) = home else {
        return Err(ConfigError::detail(
            "HOME is not set, so ~/.config/boxxycli/boxxy.toml cannot be resolved.",
        ));
    };
    Ok(PathBuf::from(home))
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    Ok(config_path_in(&home_dir()?))
}

pub fn load() -> Result<Config, ConfigError> {
    load_from(&config_path()?)
}

pub fn load_from(path: &Path) -> Result<Config, ConfigError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Err(ConfigError::Missing),
        Err(err) => {
            return Err(ConfigError::detail(format!(
                "could not read configuration: {err}"
            )));
        }
    };
    if !metadata.is_file() {
        return Err(ConfigError::detail(format!(
            "configuration path is not a file: {}",
            path.display()
        )));
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::detail(
            "configuration file is larger than 64 KiB".to_string(),
        ));
    }
    let text = fs::read_to_string(path)
        .map_err(|err| ConfigError::detail(format!("could not read configuration: {err}")))?;
    parse_config(&text)
}

pub fn generate(force: bool) -> Result<PathBuf, ConfigError> {
    let path = config_path()?;
    generate_at(&path, force)?;
    Ok(path)
}

pub fn generate_at(path: &Path, force: bool) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            ConfigError::detail(format!("could not create {}: {err}", parent.display()))
        })?;
    }

    if force {
        fs::write(path, DEFAULT_CONFIG).map_err(|err| {
            ConfigError::detail(format!("could not write {}: {err}", path.display()))
        })?;
        return Ok(());
    }

    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => file.write_all(DEFAULT_CONFIG.as_bytes()).map_err(|err| {
            ConfigError::detail(format!("could not write {}: {err}", path.display()))
        }),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(ConfigError::Exists(path.to_path_buf()))
        }
        Err(err) => Err(ConfigError::detail(format!(
            "could not write {}: {err}",
            path.display()
        ))),
    }
}

pub fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let root: toml::Value = toml::from_str(text)
        .map_err(|err| ConfigError::detail(format!("could not parse TOML: {err}")))?;
    let Some(root) = root.as_table() else {
        return Err(ConfigError::detail(
            "configuration root must be a table".to_string(),
        ));
    };

    reject_unknown(root, &["frame", "brand"], "")?;
    let frame = required_table(root, "frame", "")?;
    reject_unknown(frame, &["thickness", "radius", "color"], "frame")?;
    let brand = required_table(root, "brand", "")?;
    reject_unknown(brand, &["text"], "brand")?;

    let thickness = parse_thickness(required(frame, "thickness", "frame")?)?;
    let radius = parse_radius(required(frame, "radius", "frame")?)?;
    let color = parse_color(required(frame, "color", "frame")?)?;
    let brand = parse_brand(required(brand, "text", "brand")?)?;

    Ok(Config {
        thickness,
        radius,
        color,
        brand,
    })
}

fn reject_unknown(
    table: &toml::map::Map<String, toml::Value>,
    allowed: &[&str],
    prefix: &str,
) -> Result<(), ConfigError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ConfigError::detail(format!(
                "Unknown configuration key: {}",
                join_key(prefix, key)
            )));
        }
    }
    Ok(())
}

fn required<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
    prefix: &str,
) -> Result<&'a toml::Value, ConfigError> {
    table.get(key).ok_or_else(|| {
        ConfigError::detail(format!(
            "Missing configuration key: {}",
            join_key(prefix, key)
        ))
    })
}

fn required_table<'a>(
    table: &'a toml::map::Map<String, toml::Value>,
    key: &str,
    prefix: &str,
) -> Result<&'a toml::map::Map<String, toml::Value>, ConfigError> {
    let value = required(table, key, prefix)?;
    value.as_table().ok_or_else(|| {
        ConfigError::detail(format!(
            "Invalid configuration: {} must be a table",
            join_key(prefix, key)
        ))
    })
}

fn join_key(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

fn parse_thickness(value: &toml::Value) -> Result<Thickness, ConfigError> {
    match value {
        toml::Value::Integer(1) => Ok(Thickness::One),
        toml::Value::Integer(2) => Ok(Thickness::Two),
        toml::Value::Integer(other) => Err(ConfigError::detail(format!(
            "Invalid configuration: frame.thickness must be 1 or 2 (got {other})"
        ))),
        _ => Err(ConfigError::detail(
            "Invalid configuration: frame.thickness must be an integer".to_string(),
        )),
    }
}

fn parse_radius(value: &toml::Value) -> Result<Radius, ConfigError> {
    match value {
        toml::Value::String(text) if text == "square" => Ok(Radius::Square),
        toml::Value::String(text) if text == "rounded" => Ok(Radius::Rounded),
        toml::Value::String(text) => Err(ConfigError::detail(format!(
            "Invalid configuration: frame.radius must be \"square\" or \"rounded\" (got {text:?})"
        ))),
        _ => Err(ConfigError::detail(
            "Invalid configuration: frame.radius must be a string".to_string(),
        )),
    }
}

fn parse_color(value: &toml::Value) -> Result<Rgb, ConfigError> {
    let toml::Value::String(text) = value else {
        return Err(ConfigError::detail(
            "Invalid configuration: frame.color must be a hex RGB color like \"#89DCEB\""
                .to_string(),
        ));
    };
    parse_hex(text).ok_or_else(|| {
        ConfigError::detail(
            "Invalid configuration: frame.color must be a hex RGB color like \"#89DCEB\""
                .to_string(),
        )
    })
}

fn parse_hex(text: &str) -> Option<Rgb> {
    let hex = text.strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(Rgb { r, g, b })
}

fn parse_brand(value: &toml::Value) -> Result<String, ConfigError> {
    let toml::Value::String(text) = value else {
        return Err(ConfigError::detail(
            "Invalid configuration: brand.text must be a string".to_string(),
        ));
    };
    let chars = text.chars().count();
    if chars > MAX_BRAND_CHARS {
        return Err(ConfigError::detail(format!(
            "Invalid configuration: brand.text must be at most {MAX_BRAND_CHARS} characters (got {chars})"
        )));
    }
    if text.chars().any(is_forbidden_brand_char) {
        return Err(ConfigError::detail(
            "Invalid configuration: brand.text must not contain control characters".to_string(),
        ));
    }
    Ok(text.clone())
}

fn is_forbidden_brand_char(ch: char) -> bool {
    ch.is_control() || matches!(ch, '\u{0085}' | '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_is_valid() {
        let config = parse_config(DEFAULT_CONFIG).unwrap();
        assert_eq!(config.thickness, Thickness::One);
        assert_eq!(config.radius, Radius::Rounded);
        assert_eq!(
            config.color,
            Rgb {
                r: 0x89,
                g: 0xDC,
                b: 0xEB
            }
        );
        assert_eq!(config.brand, "BOXXY");
    }

    #[test]
    fn unknown_key_names_the_path() {
        let err = parse_config(
            "
            [frame]
            thickness = 1
            radius = \"rounded\"
            color = \"#89DCEB\"
            padding = 5

            [brand]
            text = \"BOXXY\"
            ",
        )
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Error:\nUnknown configuration key: frame.padding"
        );
    }

    #[test]
    fn unknown_root_key_and_invalid_values_fail() {
        let unknown = parse_config(&format!("{DEFAULT_CONFIG}\n[extra]\nvalue = 1\n")).unwrap_err();
        assert!(unknown
            .to_string()
            .contains("Unknown configuration key: extra"));

        let thickness = parse_config(
            "
            [frame]
            thickness = 4
            radius = \"rounded\"
            color = \"#89DCEB\"
            [brand]
            text = \"BOXXY\"
            ",
        )
        .unwrap_err();
        assert!(thickness
            .to_string()
            .contains("frame.thickness must be 1 or 2"));

        let radius = parse_config(
            "
            [frame]
            thickness = 1
            radius = \"circle\"
            color = \"#89DCEB\"
            [brand]
            text = \"BOXXY\"
            ",
        )
        .unwrap_err();
        assert!(radius.to_string().contains("frame.radius"));

        let color = parse_config(
            "
            [frame]
            thickness = 1
            radius = \"square\"
            color = \"blue\"
            [brand]
            text = \"BOXXY\"
            ",
        )
        .unwrap_err();
        assert!(color.to_string().contains("frame.color"));
    }

    #[test]
    fn brand_limit_is_characters_not_bytes() {
        let text = "デ".repeat(32);
        let ok = format!(
            "[frame]\nthickness = 2\nradius = \"square\"\ncolor = \"#aabbcc\"\n\n[brand]\ntext = \"{text}\"\n"
        );
        assert!(parse_config(&ok).is_ok());

        let too_long = "A".repeat(33);
        let bad = format!(
            "[frame]\nthickness = 1\nradius = \"rounded\"\ncolor = \"#89DCEB\"\n\n[brand]\ntext = \"{too_long}\"\n"
        );
        assert!(parse_config(&bad).unwrap_err().to_string().contains("32"));
    }

    #[test]
    fn missing_key_is_not_filled_in() {
        let err =
            parse_config("[frame]\nthickness = 1\nradius = \"rounded\"\ncolor = \"#89DCEB\"\n")
                .unwrap_err();
        assert!(err.to_string().contains("Missing configuration key: brand"));
    }

    #[test]
    fn generate_refuses_to_overwrite_unless_forced() {
        let dir = std::env::temp_dir().join(format!(
            "boxxy-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("boxxy.toml");

        generate_at(&path, false).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        assert!(matches!(
            generate_at(&path, false),
            Err(ConfigError::Exists(_))
        ));

        fs::write(&path, "stale").unwrap();
        generate_at(&path, true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_message_is_exact() {
        let dir = std::env::temp_dir().join(format!(
            "boxxy-missing-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let err = load_from(&dir.join("boxxy.toml")).unwrap_err();
        assert_eq!(
            err.to_string(),
            "No configuration found.\nRun 'boxxy --generate-config' to create one."
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
