//! 全局快捷键：用系统底层输入钩子（`inputs` 库）实现，支持任意按键、组合键、鼠标键和单独轻按修饰键。
//! 不用 RegisterHotKey 一类的系统热键，因为它们不支持鼠标键。

pub mod binding;
mod matcher;

use crate::config::Hotkeys;
use binding::{Binding, Mods, Trigger};
use inputs::{EventDisposition, EventKind, EventKinds, Listener};
use matcher::{Input, Matcher};
pub use matcher::{Mode, Signal};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Check,
    Translate,
}

/// 设置界面里的顺序。
pub const ACTIONS: [(Action, &str); 2] =
    [(Action::Check, "纠错"), (Action::Translate, "翻译 / 查词")];

pub fn get(hk: &Hotkeys, action: Action) -> &str {
    match action {
        Action::Check => &hk.check,
        Action::Translate => &hk.translate,
    }
}

pub fn set(hk: &mut Hotkeys, action: Action, spec: String) {
    match action {
        Action::Check => hk.check = spec,
        Action::Translate => hk.translate = spec,
    }
}

/// 解析全部快捷键；空字符串表示未设置。出错时返回每一项的说明。
pub fn parse_all(hk: &Hotkeys) -> Result<Vec<(Binding, Action)>, Vec<String>> {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (action, name) in ACTIONS {
        let spec = get(hk, action);
        if spec.is_empty() {
            continue;
        }
        match binding::parse(spec) {
            Ok(b) if !b.is_allowed() => errors.push(format!("{name}：不能单独使用鼠标左键或右键")),
            Ok(b) => out.push((b, action)),
            Err(e) => errors.push(format!("{name}：{e}")),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// 与其他项重复的动作。
pub fn conflicts(bindings: &[(Binding, Action)]) -> Vec<Action> {
    bindings
        .iter()
        .filter(|(b, a)| bindings.iter().any(|(b2, a2)| a2 != a && b2 == b))
        .map(|(_, a)| *a)
        .collect()
}

static MATCHER: Mutex<Matcher> = Mutex::new(Matcher::new());

fn matcher() -> std::sync::MutexGuard<'static, Matcher> {
    // 钩子回调里 panic 会被 inputs 捕获并放行，锁可能因此中毒；状态仍可用
    MATCHER.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_bindings(bindings: Vec<(Binding, Action)>) {
    matcher().set_bindings(bindings);
}

pub fn set_mode(mode: Mode) {
    matcher().set_mode(mode);
}

/// 打开时上报每次鼠标按下（`Signal::Click`），用于点击弹窗外部关闭；弹窗关着时不打扰 UI 线程。
static WATCH_CLICKS: AtomicBool = AtomicBool::new(false);

pub fn watch_clicks(on: bool) {
    WATCH_CLICKS.store(on, Ordering::Relaxed);
}

fn to_input(kind: &EventKind) -> Option<Input> {
    Some(match kind {
        EventKind::Key(k) => {
            let trigger = Trigger::Key(k.physical);
            if k.state.is_pressed() {
                Input::Down {
                    trigger,
                    mods: Some(Mods::from(k.modifiers)),
                    repeat: k.repeat,
                }
            } else {
                Input::Up { trigger }
            }
        }
        EventKind::Button(b) => {
            let trigger = Trigger::Mouse(b.button);
            if b.state.is_pressed() {
                // 鼠标事件不带修饰键状态，由 Matcher 按自己跟踪的状态补上
                Input::Down {
                    trigger,
                    mods: None,
                    repeat: false,
                }
            } else {
                Input::Up { trigger }
            }
        }
        EventKind::CaptureInterrupted => Input::Interrupted,
        _ => return None,
    })
}

/// 启动全局输入钩子。返回的 Listener 需要一直持有，丢弃即停止。
/// on_signal 在钩子线程上调用，应尽快把工作转交给 UI 线程。
pub fn start(on_signal: impl Fn(Signal) + Send + Sync + 'static) -> Result<Listener, String> {
    Listener::builder()
        // 不要鼠标移动和滚轮，减少回调次数
        .event_kinds(EventKinds::KEY.union(EventKinds::BUTTON))
        .start(move |event: &inputs::Event| {
            // 别的程序（包括我们自己用 enigo 模拟的 Ctrl+C）合成的输入不参与匹配
            if event.injected {
                return EventDisposition::Pass;
            }
            let Some(input) = to_input(&event.kind) else {
                return EventDisposition::Pass;
            };
            let mouse_down = matches!(
                input,
                Input::Down {
                    trigger: Trigger::Mouse(_),
                    ..
                }
            );
            let (suppress, signal) = matcher().handle(input);
            if let Some(s) = signal {
                on_signal(s);
            } else if mouse_down && !suppress && WATCH_CLICKS.load(Ordering::Relaxed) {
                on_signal(Signal::Click);
            }
            if suppress {
                EventDisposition::Suppress
            } else {
                EventDisposition::Pass
            }
        })
        .map_err(|e| format!("无法监听全局按键：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_and_conflicts() {
        let mut hk = Hotkeys::default();
        let bindings = parse_all(&hk).unwrap();
        assert_eq!(bindings.len(), 2);
        assert!(conflicts(&bindings).is_empty());

        hk.translate = hk.check.clone();
        assert_eq!(
            conflicts(&parse_all(&hk).unwrap()),
            [Action::Check, Action::Translate]
        );

        hk.translate = "MouseLeft".into();
        hk.check = "Nope+KeyG".into();
        assert_eq!(parse_all(&hk).unwrap_err().len(), 2);
    }
}
