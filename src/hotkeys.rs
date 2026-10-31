//! 快捷键：配置字符串（global-hotkey 格式，如 `CmdOrCtrl+Alt+G`）的录制、显示和注册。

use crate::config::Hotkeys;
use global_hotkey::{GlobalHotKeyManager, hotkey::HotKey};
use slint::platform::Key;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    Check,
    Fix,
    Translate,
}

/// 设置界面里的顺序，也是 Hotkeys 各字段的顺序。
pub const ACTIONS: [(Action, &str); 3] = [
    (Action::Check, "纠错"),
    (Action::Fix, "一键修正"),
    (Action::Translate, "翻译 / 查词"),
];

pub fn get(hk: &Hotkeys, action: Action) -> &str {
    match action {
        Action::Check => &hk.check,
        Action::Fix => &hk.fix,
        Action::Translate => &hk.translate,
    }
}

pub fn set(hk: &mut Hotkeys, action: Action, spec: String) {
    match action {
        Action::Check => hk.check = spec,
        Action::Fix => hk.fix = spec,
        Action::Translate => hk.translate = spec,
    }
}

/// Slint 的修饰键语义：macOS 上 control 是 ⌘，meta 是 Control；Windows 上 meta 是 Win 键。
#[derive(Clone, Copy, Default)]
pub struct Mods {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

const META_TOKEN: &str = if cfg!(target_os = "macos") {
    "Ctrl"
} else {
    "Super"
};

fn main_key(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    if c.is_ascii_alphanumeric() {
        return Some(c.to_ascii_uppercase().to_string());
    }
    if c == ' ' {
        return Some("Space".into());
    }
    const F_KEYS: [Key; 12] = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    F_KEYS
        .iter()
        .position(|k| char::from(*k) == c)
        .map(|i| format!("F{}", i + 1))
}

/// 把录制到的按键转成配置字符串。只按了修饰键、没有 Ctrl/Alt/Win 或按键不支持时返回 None。
pub fn record(text: &str, m: Mods) -> Option<String> {
    if !(m.control || m.alt || m.meta) {
        return None;
    }
    let key = main_key(text)?;
    let mut parts: Vec<&str> = Vec::with_capacity(5);
    if m.control {
        parts.push("CmdOrCtrl");
    }
    if m.meta {
        parts.push(META_TOKEN);
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Shift");
    }
    parts.push(&key);
    Some(parts.join("+"))
}

/// 配置字符串转成界面上显示的样子。
pub fn display(spec: &str) -> String {
    if spec.is_empty() {
        return "未设置".into();
    }
    let mac = cfg!(target_os = "macos");
    spec.split('+')
        .map(|t| match (t.to_ascii_lowercase().as_str(), mac) {
            ("cmdorctrl", true) => "⌘",
            ("cmdorctrl", false) => "Ctrl",
            ("ctrl" | "control", true) => "⌃",
            ("alt" | "option", true) => "⌥",
            ("shift", true) => "⇧",
            ("super", false) => "Win",
            _ => t,
        })
        .collect::<Vec<_>>()
        .join(if mac { " " } else { " + " })
}

/// 返回与其他项重复的动作（空字符串不算）。
pub fn conflicts(hk: &Hotkeys) -> Vec<Action> {
    ACTIONS
        .iter()
        .map(|(a, _)| *a)
        .filter(|&a| {
            let spec = get(hk, a);
            !spec.is_empty()
                && ACTIONS
                    .iter()
                    .any(|(b, _)| *b != a && get(hk, *b).eq_ignore_ascii_case(spec))
        })
        .collect()
}

pub struct Registry {
    manager: GlobalHotKeyManager,
    active: Vec<(HotKey, Action)>,
}

impl Registry {
    pub fn new() -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| e.to_string())?;
        Ok(Self {
            manager,
            active: Vec::new(),
        })
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        self.active
            .iter()
            .find(|(hk, _)| hk.id() == id)
            .map(|(_, a)| *a)
    }

    /// 注销全部（录制快捷键时要先注销，否则按键会被全局快捷键拦走）。
    pub fn clear(&mut self) {
        for (hk, _) in self.active.drain(..) {
            let _ = self.manager.unregister(hk);
        }
    }

    /// 按配置重新注册，返回失败项的说明。
    pub fn apply(&mut self, hk: &Hotkeys) -> Vec<String> {
        self.clear();
        let mut errors = Vec::new();
        for (action, name) in ACTIONS {
            let spec = get(hk, action);
            if spec.is_empty() {
                continue;
            }
            let result = spec
                .parse::<HotKey>()
                .map_err(|e| e.to_string())
                .and_then(|h| {
                    self.manager
                        .register(h)
                        .map(|()| h)
                        .map_err(|e| e.to_string())
                });
            match result {
                Ok(h) => self.active.push((h, action)),
                Err(e) => errors.push(format!("{name}（{}）：{e}", display(spec))),
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CTRL_ALT: Mods = Mods {
        control: true,
        alt: true,
        shift: false,
        meta: false,
    };

    #[test]
    fn record_letter_and_digit() {
        assert_eq!(record("g", CTRL_ALT).as_deref(), Some("CmdOrCtrl+Alt+G"));
        assert_eq!(record("7", CTRL_ALT).as_deref(), Some("CmdOrCtrl+Alt+7"));
        let all = Mods {
            shift: true,
            ..CTRL_ALT
        };
        assert_eq!(record("G", all).as_deref(), Some("CmdOrCtrl+Alt+Shift+G"));
    }

    #[test]
    fn record_function_key_and_space() {
        let f5: String = char::from(Key::F5).into();
        assert_eq!(record(&f5, CTRL_ALT).as_deref(), Some("CmdOrCtrl+Alt+F5"));
        assert_eq!(
            record(" ", CTRL_ALT).as_deref(),
            Some("CmdOrCtrl+Alt+Space")
        );
    }

    #[test]
    fn record_rejects_incomplete() {
        assert_eq!(record("g", Mods::default()), None);
        assert_eq!(
            record(
                "g",
                Mods {
                    shift: true,
                    ..Default::default()
                }
            ),
            None
        );
        // 只按了修饰键时 text 是修饰键本身的特殊字符
        let ctrl: String = char::from(Key::Control).into();
        assert_eq!(record(&ctrl, CTRL_ALT), None);
    }

    #[test]
    fn recorded_specs_parse() {
        for spec in [
            "CmdOrCtrl+Alt+G",
            "CmdOrCtrl+Alt+Shift+F5",
            "CmdOrCtrl+Alt+Space",
            "Super+Alt+7",
        ] {
            assert!(spec.parse::<HotKey>().is_ok(), "{spec}");
        }
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn display_windows() {
        assert_eq!(display("CmdOrCtrl+Alt+Shift+G"), "Ctrl + Alt + Shift + G");
        assert_eq!(display("Super+Alt+T"), "Win + Alt + T");
        assert_eq!(display(""), "未设置");
    }

    #[test]
    fn detects_conflicts() {
        let mut hk = Hotkeys::default();
        assert!(conflicts(&hk).is_empty());
        hk.translate = hk.check.to_lowercase();
        assert_eq!(conflicts(&hk), [Action::Check, Action::Translate]);
        hk.check.clear();
        hk.translate.clear();
        assert!(conflicts(&hk).is_empty());
    }
}
