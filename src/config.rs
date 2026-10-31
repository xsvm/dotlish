use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub hotkeys: Hotkeys,
    /// 为空时读取环境变量 HTTPS_PROXY / HTTP_PROXY / ALL_PROXY
    pub proxy: String,
    /// 选中内容不是中文时翻译成这个语言
    pub target_lang: String,
}

/// 写法见 hotkeys::binding，例如 `Ctrl+Alt+KeyG`、`MouseBack`；空字符串表示未设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub check: String,
    pub translate: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkeys: Hotkeys::default(),
            proxy: String::new(),
            target_lang: "zh-CN".into(),
        }
    }
}

impl Default for Hotkeys {
    fn default() -> Self {
        // macOS 习惯用 ⌘（Meta），其他平台用 Ctrl
        let m = if cfg!(target_os = "macos") {
            "Meta"
        } else {
            "Ctrl"
        };
        Self {
            check: format!("{m}+Alt+KeyG"),
            translate: format!("{m}+Alt+KeyT"),
        }
    }
}

const APP_DIR: &str = "dotlish";
/// 改名前的配置目录
const OLD_APP_DIR: &str = "select-assist";

pub fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join(APP_DIR).join("config.toml"))
}

/// 改名前的配置目录整个搬过来，快捷键等设置不丢。
fn migrate() {
    let Some(base) = dirs::config_dir() else {
        return;
    };
    let (old, new) = (base.join(OLD_APP_DIR), base.join(APP_DIR));
    if old.exists() && !new.exists() {
        let _ = std::fs::rename(old, new);
    }
}

/// 读取配置；文件不存在时写入默认配置，方便用户找到并修改。
pub fn load() -> Config {
    migrate();
    let Some(path) = path() else {
        return Config::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("配置文件 {} 格式错误，使用默认配置：{e}", path.display());
            Config::default()
        }),
        Err(_) => {
            let cfg = Config::default();
            let _ = save(&cfg);
            cfg
        }
    }
}

pub fn save(cfg: &Config) -> Result<(), String> {
    let path = path().ok_or("找不到配置目录")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let s = toml::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, s).map_err(|e| format!("保存配置失败：{e}"))
}
