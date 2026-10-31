//! 快捷键的数据模型与配置字符串格式（纯逻辑，可测试）。
//!
//! 格式：修饰键与触发键用 `+` 连接，例如 `Ctrl+Alt+KeyG`、`Ctrl+MouseBack`、`F8`、`ControlRight`。
//! 触发键名沿用 `inputs::KeyCode` 的变体名；无法映射的键写成 `Raw:Windows:93` 这种原生码。

use inputs::{Button, KeyCode, NativeKeyCode, PhysicalKey};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Windows 上是 Win 键，macOS 上是 ⌘
    pub meta: bool,
}

impl Mods {
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

impl From<inputs::Modifiers> for Mods {
    fn from(m: inputs::Modifiers) -> Self {
        Self {
            ctrl: m.control,
            alt: m.alt,
            shift: m.shift,
            meta: m.meta,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    Key(PhysicalKey),
    Mouse(Button),
}

impl Trigger {
    /// 修饰键本身作为触发键时，只在"单独轻按后松开"时触发。
    pub fn modifier_flag(self) -> Option<Mods> {
        let Trigger::Key(PhysicalKey::Code(k)) = self else {
            return None;
        };
        use KeyCode::*;
        Some(match k {
            ControlLeft | ControlRight => Mods {
                ctrl: true,
                ..Mods::default()
            },
            AltLeft | AltRight => Mods {
                alt: true,
                ..Mods::default()
            },
            ShiftLeft | ShiftRight => Mods {
                shift: true,
                ..Mods::default()
            },
            MetaLeft | MetaRight => Mods {
                meta: true,
                ..Mods::default()
            },
            _ => return None,
        })
    }

    /// 单独按会影响正常打字的键。
    pub fn is_typing_key(self) -> bool {
        let Trigger::Key(PhysicalKey::Code(k)) = self else {
            return false;
        };
        use KeyCode::*;
        matches!(
            k,
            KeyA | KeyB
                | KeyC
                | KeyD
                | KeyE
                | KeyF
                | KeyG
                | KeyH
                | KeyI
                | KeyJ
                | KeyK
                | KeyL
                | KeyM
                | KeyN
                | KeyO
                | KeyP
                | KeyQ
                | KeyR
                | KeyS
                | KeyT
                | KeyU
                | KeyV
                | KeyW
                | KeyX
                | KeyY
                | KeyZ
                | Digit0
                | Digit1
                | Digit2
                | Digit3
                | Digit4
                | Digit5
                | Digit6
                | Digit7
                | Digit8
                | Digit9
                | Backquote
                | Minus
                | Equal
                | BracketLeft
                | BracketRight
                | Backslash
                | Semicolon
                | Quote
                | Comma
                | Period
                | Slash
                | Space
                | Enter
                | Backspace
                | Tab
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub mods: Mods,
    pub trigger: Trigger,
}

impl Binding {
    /// 不带修饰键的左右键会让鼠标没法正常用，不允许。
    pub fn is_allowed(self) -> bool {
        !(self.mods.is_empty()
            && matches!(self.trigger, Trigger::Mouse(Button::Left | Button::Right)))
    }

    /// 会干扰正常使用时给出提示。
    pub fn warning(self) -> Option<&'static str> {
        (self.mods.is_empty() && self.trigger.is_typing_key()).then_some("会影响打字")
    }
}

type SetMod = fn(&mut Mods);

const MOD_TOKENS: [(&str, SetMod); 4] = [
    ("Ctrl", |m| m.ctrl = true),
    ("Alt", |m| m.alt = true),
    ("Shift", |m| m.shift = true),
    ("Meta", |m| m.meta = true),
];

fn key_name(k: KeyCode) -> String {
    format!("{k:?}")
}

fn parse_key_name(s: &str) -> Option<KeyCode> {
    serde_json::from_value(serde_json::Value::String(s.into())).ok()
}

fn trigger_token(t: Trigger) -> String {
    match t {
        Trigger::Key(PhysicalKey::Code(k)) => key_name(k),
        Trigger::Key(PhysicalKey::Native(n)) => match n {
            NativeKeyCode::Windows(c) => format!("Raw:Windows:{c}"),
            NativeKeyCode::MacOs(c) => format!("Raw:MacOs:{c}"),
            NativeKeyCode::Evdev(c) => format!("Raw:Evdev:{c}"),
            NativeKeyCode::Xkb(c) => format!("Raw:Xkb:{c}"),
        },
        Trigger::Mouse(b) => match b {
            Button::Left => "MouseLeft".into(),
            Button::Right => "MouseRight".into(),
            Button::Middle => "MouseMiddle".into(),
            Button::Back => "MouseBack".into(),
            Button::Forward => "MouseForward".into(),
            Button::Other(i) => format!("Mouse:{i}"),
            _ => format!("Mouse:{b:?}"),
        },
    }
}

fn parse_trigger(s: &str) -> Option<Trigger> {
    let mouse = |b| Some(Trigger::Mouse(b));
    match s {
        "MouseLeft" => return mouse(Button::Left),
        "MouseRight" => return mouse(Button::Right),
        "MouseMiddle" => return mouse(Button::Middle),
        "MouseBack" => return mouse(Button::Back),
        "MouseForward" => return mouse(Button::Forward),
        _ => {}
    }
    if let Some(i) = s.strip_prefix("Mouse:") {
        return mouse(Button::Other(i.parse().ok()?));
    }
    if let Some(raw) = s.strip_prefix("Raw:") {
        let (os, code) = raw.split_once(':')?;
        let native = match os {
            "Windows" => NativeKeyCode::Windows(code.parse().ok()?),
            "MacOs" => NativeKeyCode::MacOs(code.parse().ok()?),
            "Evdev" => NativeKeyCode::Evdev(code.parse().ok()?),
            "Xkb" => NativeKeyCode::Xkb(code.parse().ok()?),
            _ => return None,
        };
        return Some(Trigger::Key(PhysicalKey::Native(native)));
    }
    // 兼容旧写法：单个字母或数字
    let key = parse_key_name(s).or_else(|| match s.as_bytes() {
        [c] if c.is_ascii_alphabetic() => {
            parse_key_name(&format!("Key{}", c.to_ascii_uppercase() as char))
        }
        [c] if c.is_ascii_digit() => parse_key_name(&format!("Digit{}", *c as char)),
        _ => None,
    })?;
    Some(Trigger::Key(PhysicalKey::Code(key)))
}

pub fn format(b: Binding) -> String {
    let mut parts: Vec<String> = MOD_TOKENS
        .iter()
        .zip([b.mods.ctrl, b.mods.alt, b.mods.shift, b.mods.meta])
        .filter(|(_, on)| *on)
        .map(|((name, _), _)| (*name).to_string())
        .collect();
    parts.push(trigger_token(b.trigger));
    parts.join("+")
}

pub fn parse(spec: &str) -> Result<Binding, String> {
    let mut mods = Mods::default();
    let mut tokens: Vec<&str> = spec.split('+').map(str::trim).collect();
    let last = tokens
        .pop()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("“{spec}”缺少按键"))?;
    for t in tokens {
        // 兼容旧写法 CmdOrCtrl
        let t = match t {
            "CmdOrCtrl" if cfg!(target_os = "macos") => "Meta",
            "CmdOrCtrl" => "Ctrl",
            t => t,
        };
        let (_, set) = MOD_TOKENS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(t))
            .ok_or_else(|| format!("“{spec}”里的“{t}”不是修饰键"))?;
        set(&mut mods);
    }
    let trigger = parse_trigger(last).ok_or_else(|| format!("“{spec}”里的“{last}”无法识别"))?;
    Ok(Binding { mods, trigger })
}

fn display_trigger(t: Trigger) -> String {
    let mac = cfg!(target_os = "macos");
    match t {
        Trigger::Mouse(b) => match b {
            Button::Left => "鼠标左键".into(),
            Button::Right => "鼠标右键".into(),
            Button::Middle => "鼠标中键".into(),
            Button::Back => "鼠标后退键".into(),
            Button::Forward => "鼠标前进键".into(),
            Button::Other(i) => format!("鼠标键 {i}"),
            _ => format!("鼠标 {b:?}"),
        },
        Trigger::Key(PhysicalKey::Native(n)) => format!("键码 {n:?}"),
        Trigger::Key(PhysicalKey::Code(k)) => {
            use KeyCode::*;
            let named = match k {
                ControlLeft => {
                    if mac {
                        "左 ⌃"
                    } else {
                        "左 Ctrl"
                    }
                }
                ControlRight => {
                    if mac {
                        "右 ⌃"
                    } else {
                        "右 Ctrl"
                    }
                }
                AltLeft => {
                    if mac {
                        "左 ⌥"
                    } else {
                        "左 Alt"
                    }
                }
                AltRight => {
                    if mac {
                        "右 ⌥"
                    } else {
                        "右 Alt"
                    }
                }
                ShiftLeft => "左 Shift",
                ShiftRight => "右 Shift",
                MetaLeft => {
                    if mac {
                        "左 ⌘"
                    } else {
                        "左 Win"
                    }
                }
                MetaRight => {
                    if mac {
                        "右 ⌘"
                    } else {
                        "右 Win"
                    }
                }
                Space => "空格",
                ArrowUp => "↑",
                ArrowDown => "↓",
                ArrowLeft => "←",
                ArrowRight => "→",
                _ => "",
            };
            if !named.is_empty() {
                return named.into();
            }
            let name = key_name(k);
            name.strip_prefix("Key")
                .or_else(|| name.strip_prefix("Digit"))
                .map(String::from)
                .unwrap_or(name)
        }
    }
}

/// 界面上显示的样子，例如 `Ctrl + Alt + G`、`⌘ ⌥ G`、`鼠标后退键`。
pub fn display(b: Binding) -> String {
    let mac = cfg!(target_os = "macos");
    let mods = [
        (b.mods.ctrl, if mac { "⌃" } else { "Ctrl" }),
        (b.mods.alt, if mac { "⌥" } else { "Alt" }),
        (b.mods.shift, if mac { "⇧" } else { "Shift" }),
        (b.mods.meta, if mac { "⌘" } else { "Win" }),
    ];
    let mut parts: Vec<String> = mods
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| (*n).to_string())
        .collect();
    parts.push(display_trigger(b.trigger));
    parts.join(if mac { " " } else { " + " })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(k: KeyCode) -> Trigger {
        Trigger::Key(PhysicalKey::Code(k))
    }

    #[test]
    fn round_trip() {
        for spec in [
            "Ctrl+Alt+KeyG",
            "Ctrl+Alt+Shift+F5",
            "Meta+Space",
            "F8",
            "ControlRight",
            "Ctrl+MouseLeft",
            "MouseBack",
            "Shift+Mouse:7",
            "Alt+Raw:Windows:93",
        ] {
            let b = parse(spec).unwrap();
            assert_eq!(format(b), spec);
        }
    }

    #[test]
    fn legacy_specs() {
        let b = parse("CmdOrCtrl+Alt+G").unwrap();
        assert_eq!(b.trigger, key(KeyCode::KeyG));
        assert!(b.mods.alt);
        assert_eq!(parse("Ctrl+7").unwrap().trigger, key(KeyCode::Digit7));
    }

    #[test]
    fn rejects_bad_specs() {
        assert!(parse("").is_err());
        assert!(parse("Ctrl+").is_err());
        assert!(parse("Hyper+KeyG").is_err());
        assert!(parse("Ctrl+NoSuchKey").is_err());
    }

    #[test]
    fn plain_left_click_not_allowed() {
        assert!(!parse("MouseLeft").unwrap().is_allowed());
        assert!(parse("Ctrl+MouseLeft").unwrap().is_allowed());
        assert!(parse("MouseBack").unwrap().is_allowed());
    }

    #[test]
    fn typing_warning() {
        assert!(parse("KeyG").unwrap().warning().is_some());
        assert!(parse("Ctrl+KeyG").unwrap().warning().is_none());
        assert!(parse("F8").unwrap().warning().is_none());
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn display_windows() {
        assert_eq!(
            display(parse("Ctrl+Alt+Shift+KeyG").unwrap()),
            "Ctrl + Alt + Shift + G"
        );
        assert_eq!(display(parse("Meta+Digit3").unwrap()), "Win + 3");
        assert_eq!(display(parse("MouseBack").unwrap()), "鼠标后退键");
        assert_eq!(display(parse("ControlRight").unwrap()), "右 Ctrl");
    }
}
