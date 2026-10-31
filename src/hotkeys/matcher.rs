//! 按键匹配状态机。只处理抽象后的输入，不碰系统 API，可直接单元测试。
//!
//! 运行在系统输入钩子的回调里，必须很快：每个事件只做 O(绑定数) 的线性扫描（绑定只有几个）。

use super::Action;
use super::binding::{Binding, Mods, Trigger};
use inputs::{KeyCode, PhysicalKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// 匹配并触发
    Normal,
    /// 全部放行（设置窗口打开时）
    Paused,
    /// 下一个按键或鼠标键作为新快捷键
    Recording,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// mods 为 None 时（鼠标事件）用自己跟踪的修饰键状态
    Down {
        trigger: Trigger,
        mods: Option<Mods>,
        repeat: bool,
    },
    Up {
        trigger: Trigger,
    },
    /// 系统中断了捕获（比如 macOS 超时关掉了钩子），清空按住状态
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Fire(Action),
    Recorded(Binding),
    RecordCancelled,
    /// 鼠标在别处按下（不是快捷键）。由 `hotkeys::start` 产生，不经过 Matcher。
    Click,
}

pub struct Matcher {
    bindings: Vec<(Binding, Action)>,
    mode: Mode,
    /// 按下时被拦截、还没松开的键；松开事件也要拦截，避免目标程序收到孤立的抬起
    held: Vec<Trigger>,
    /// 单独按下、期间没按别的键的修饰键
    tap: Option<Trigger>,
    /// 当前按住的修饰键，每个物理键一位（左右分开）
    pressed: u8,
}

fn modifier_bit(t: Trigger) -> Option<u8> {
    let Trigger::Key(PhysicalKey::Code(k)) = t else {
        return None;
    };
    use KeyCode::*;
    let i = match k {
        ControlLeft => 0,
        ControlRight => 1,
        AltLeft => 2,
        AltRight => 3,
        ShiftLeft => 4,
        ShiftRight => 5,
        MetaLeft => 6,
        MetaRight => 7,
        _ => return None,
    };
    Some(1 << i)
}

fn mods_from_bits(b: u8) -> Mods {
    Mods {
        ctrl: b & 0b11 != 0,
        alt: b & 0b1100 != 0,
        shift: b & 0b11_0000 != 0,
        meta: b & 0b1100_0000 != 0,
    }
}

const ESCAPE: Trigger = Trigger::Key(PhysicalKey::Code(KeyCode::Escape));

impl Matcher {
    pub const fn new() -> Self {
        Self {
            bindings: Vec::new(),
            mode: Mode::Paused,
            held: Vec::new(),
            tap: None,
            pressed: 0,
        }
    }

    pub fn set_bindings(&mut self, bindings: Vec<(Binding, Action)>) {
        self.bindings = bindings;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
    }

    fn lookup(&self, b: Binding) -> Option<Action> {
        self.bindings.iter().find(|(x, _)| *x == b).map(|(_, a)| *a)
    }

    /// 返回 (是否拦截, 要通知 UI 的信号)。
    pub fn handle(&mut self, input: Input) -> (bool, Option<Signal>) {
        match input {
            Input::Interrupted => {
                self.held.clear();
                self.tap = None;
                self.pressed = 0;
                (false, None)
            }
            Input::Down {
                trigger,
                mods,
                repeat,
            } => self.down(trigger, mods, repeat),
            Input::Up { trigger } => self.up(trigger),
        }
    }

    fn down(
        &mut self,
        trigger: Trigger,
        mods: Option<Mods>,
        repeat: bool,
    ) -> (bool, Option<Signal>) {
        if self.held.contains(&trigger) {
            return (true, None);
        }
        let mods = mods.unwrap_or_else(|| mods_from_bits(self.pressed));
        if let Some(own) = trigger.modifier_flag() {
            self.pressed |= modifier_bit(trigger).unwrap_or(0);
            if !repeat {
                // 各平台对"修饰键自身按下时 mods 里是否已包含它"不一致，去掉自身再判断
                let others = Mods {
                    ctrl: mods.ctrl && !own.ctrl,
                    alt: mods.alt && !own.alt,
                    shift: mods.shift && !own.shift,
                    meta: mods.meta && !own.meta,
                };
                self.tap = others.is_empty().then_some(trigger);
            }
            return (false, None);
        }
        self.tap = None;
        let binding = Binding { mods, trigger };
        match self.mode {
            Mode::Paused => (false, None),
            Mode::Recording if trigger == ESCAPE && mods.is_empty() => {
                self.mode = Mode::Paused;
                self.held.push(trigger);
                (true, Some(Signal::RecordCancelled))
            }
            // 单独的左右键放行，用户还要点界面
            Mode::Recording if !binding.is_allowed() => (false, None),
            Mode::Recording => {
                self.mode = Mode::Paused;
                self.held.push(trigger);
                (true, Some(Signal::Recorded(binding)))
            }
            Mode::Normal => match self.lookup(binding) {
                Some(a) => {
                    self.held.push(trigger);
                    (true, Some(Signal::Fire(a)))
                }
                None => (false, None),
            },
        }
    }

    fn up(&mut self, trigger: Trigger) -> (bool, Option<Signal>) {
        self.pressed &= !modifier_bit(trigger).unwrap_or(0);
        if let Some(i) = self.held.iter().position(|t| *t == trigger) {
            self.held.swap_remove(i);
            return (true, None);
        }
        if self.tap != Some(trigger) {
            return (false, None);
        }
        // 修饰键单独轻按：按下和松开都已放行，这里只发信号
        self.tap = None;
        let binding = Binding {
            mods: Mods::default(),
            trigger,
        };
        let signal = match self.mode {
            Mode::Recording => {
                self.mode = Mode::Paused;
                Some(Signal::Recorded(binding))
            }
            Mode::Normal => self.lookup(binding).map(Signal::Fire),
            Mode::Paused => None,
        };
        (false, signal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkeys::binding::parse;
    use inputs::Button;

    fn key(k: KeyCode) -> Trigger {
        Trigger::Key(PhysicalKey::Code(k))
    }
    const CTRL_ALT: Mods = Mods {
        ctrl: true,
        alt: true,
        shift: false,
        meta: false,
    };
    const CTRL: Mods = Mods {
        ctrl: true,
        alt: false,
        shift: false,
        meta: false,
    };

    fn down(t: Trigger, mods: Mods) -> Input {
        Input::Down {
            trigger: t,
            mods: Some(mods),
            repeat: false,
        }
    }

    fn click(b: Button) -> Input {
        Input::Down {
            trigger: Trigger::Mouse(b),
            mods: None,
            repeat: false,
        }
    }

    fn matcher(specs: &[(&str, Action)]) -> Matcher {
        let mut m = Matcher::new();
        m.set_bindings(specs.iter().map(|(s, a)| (parse(s).unwrap(), *a)).collect());
        m.set_mode(Mode::Normal);
        m
    }

    #[test]
    fn fires_and_suppresses_press_repeat_and_release() {
        let mut m = matcher(&[("Ctrl+Alt+KeyG", Action::Check)]);
        let g = key(KeyCode::KeyG);
        assert_eq!(
            m.handle(down(g, CTRL_ALT)),
            (true, Some(Signal::Fire(Action::Check)))
        );
        assert_eq!(
            m.handle(Input::Down {
                trigger: g,
                mods: Some(CTRL_ALT),
                repeat: true
            }),
            (true, None)
        );
        assert_eq!(m.handle(Input::Up { trigger: g }), (true, None));
        // 松开后再按普通的 G 放行
        assert_eq!(m.handle(down(g, Mods::default())), (false, None));
    }

    #[test]
    fn mods_must_match_exactly() {
        let mut m = matcher(&[("Ctrl+KeyG", Action::Check)]);
        let g = key(KeyCode::KeyG);
        assert_eq!(m.handle(down(g, CTRL_ALT)), (false, None));
        assert_eq!(m.handle(down(g, CTRL)).1, Some(Signal::Fire(Action::Check)));
    }

    #[test]
    fn mouse_button() {
        let mut m = matcher(&[("MouseBack", Action::Translate)]);
        let back = Trigger::Mouse(Button::Back);
        assert_eq!(
            m.handle(click(Button::Back)),
            (true, Some(Signal::Fire(Action::Translate)))
        );
        assert_eq!(m.handle(Input::Up { trigger: back }), (true, None));
    }

    #[test]
    fn mouse_uses_tracked_modifiers() {
        let mut m = matcher(&[("Ctrl+MouseMiddle", Action::Check)]);
        let lctrl = key(KeyCode::ControlLeft);
        assert_eq!(m.handle(click(Button::Middle)), (false, None));
        m.handle(Input::Up {
            trigger: Trigger::Mouse(Button::Middle),
        });
        m.handle(down(lctrl, CTRL));
        assert_eq!(
            m.handle(click(Button::Middle)).1,
            Some(Signal::Fire(Action::Check))
        );
        m.handle(Input::Up {
            trigger: Trigger::Mouse(Button::Middle),
        });
        m.handle(Input::Up { trigger: lctrl });
        assert_eq!(m.handle(click(Button::Middle)), (false, None));
    }

    #[test]
    fn lone_modifier_tap() {
        let mut m = matcher(&[("ControlRight", Action::Translate)]);
        let rctrl = key(KeyCode::ControlRight);
        // 按下时 mods 里已含自身也算单独
        assert_eq!(m.handle(down(rctrl, CTRL)), (false, None));
        assert_eq!(
            m.handle(Input::Up { trigger: rctrl }),
            (false, Some(Signal::Fire(Action::Translate)))
        );
    }

    #[test]
    fn modifier_used_in_combo_is_not_a_tap() {
        let mut m = matcher(&[("ControlRight", Action::Translate)]);
        let rctrl = key(KeyCode::ControlRight);
        m.handle(down(rctrl, CTRL));
        m.handle(down(key(KeyCode::KeyC), CTRL));
        m.handle(Input::Up {
            trigger: key(KeyCode::KeyC),
        });
        assert_eq!(m.handle(Input::Up { trigger: rctrl }), (false, None));
    }

    #[test]
    fn recording() {
        let mut m = matcher(&[]);
        m.set_mode(Mode::Recording);
        // 单独左键放行，继续录制
        assert_eq!(m.handle(click(Button::Left)), (false, None));
        let f8 = key(KeyCode::F8);
        let expected = Binding {
            mods: Mods::default(),
            trigger: f8,
        };
        assert_eq!(
            m.handle(down(f8, Mods::default())),
            (true, Some(Signal::Recorded(expected)))
        );
        // 录完回到暂停，不触发任何东西
        assert_eq!(m.handle(Input::Up { trigger: f8 }), (true, None));
        assert_eq!(m.handle(down(f8, Mods::default())), (false, None));
    }

    #[test]
    fn recording_escape_cancels_and_modifier_tap_records() {
        let mut m = matcher(&[]);
        m.set_mode(Mode::Recording);
        let esc = key(KeyCode::Escape);
        assert_eq!(
            m.handle(down(esc, Mods::default())),
            (true, Some(Signal::RecordCancelled))
        );
        m.handle(Input::Up { trigger: esc });

        m.set_mode(Mode::Recording);
        let ralt = key(KeyCode::AltRight);
        m.handle(down(
            ralt,
            Mods {
                alt: true,
                ..Mods::default()
            },
        ));
        let expected = Binding {
            mods: Mods::default(),
            trigger: ralt,
        };
        assert_eq!(
            m.handle(Input::Up { trigger: ralt }),
            (false, Some(Signal::Recorded(expected)))
        );
    }

    #[test]
    fn interrupted_clears_held() {
        let mut m = matcher(&[("F8", Action::Check)]);
        let f8 = key(KeyCode::F8);
        m.handle(down(f8, Mods::default()));
        m.handle(Input::Interrupted);
        assert_eq!(m.handle(Input::Up { trigger: f8 }), (false, None));
    }
}
