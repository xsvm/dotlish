//! 和其他程序交互：模拟复制/粘贴读写选中文本（用完恢复剪贴板），读取光标位置。
//! 只在 UI 线程调用。

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Mouse, Settings};
use std::cell::RefCell;
use std::thread::sleep;
use std::time::Duration;

#[cfg(target_os = "macos")]
mod keys {
    use enigo::Key;
    pub const MODIFIER: Key = Key::Meta;
    pub const C: Key = Key::Unicode('c');
    pub const V: Key = Key::Unicode('v');
}
#[cfg(not(target_os = "macos"))]
mod keys {
    use enigo::Key;
    pub const MODIFIER: Key = Key::Control;
    pub const C: Key = Key::C;
    pub const V: Key = Key::V;
}

thread_local! {
    // 复用同一个连接，不必每次按快捷键都新建
    static ENIGO: RefCell<Option<Enigo>> = const { RefCell::new(None) };
}

fn with_enigo<R>(f: impl FnOnce(&mut Enigo) -> Option<R>) -> Option<R> {
    ENIGO.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Enigo::new(&Settings::default()).ok();
        }
        f(slot.as_mut()?)
    })
}

fn chord(key: Key) -> Option<()> {
    with_enigo(|enigo| {
        // 用户可能还按着快捷键里的 Alt/Shift，不松开的话发出去的是 Ctrl+Alt+C
        for k in [Key::Alt, Key::Shift] {
            let _ = enigo.key(k, Direction::Release);
        }
        enigo.key(keys::MODIFIER, Direction::Press).ok()?;
        let r = enigo.key(key, Direction::Click);
        enigo.key(keys::MODIFIER, Direction::Release).ok()?;
        r.ok()
    })
}

pub fn cursor() -> Option<(i32, i32)> {
    with_enigo(|enigo| enigo.location().ok())
}

fn restore(cb: &mut Clipboard, saved: Option<String>) {
    let _ = match saved {
        Some(s) => cb.set_text(s),
        None => cb.clear(),
    };
}

/// 读取当前选中的文本；没有选中内容时返回 None。
pub fn grab_selection() -> Option<String> {
    let mut cb = Clipboard::new().ok()?;
    let saved = cb.get_text().ok();
    let _ = cb.clear();
    chord(keys::C)?;

    let mut text = None;
    for _ in 0..20 {
        sleep(Duration::from_millis(25));
        if let Ok(t) = cb.get_text()
            && !t.trim().is_empty()
        {
            text = Some(t);
            break;
        }
    }
    restore(&mut cb, saved);
    text
}

/// 用 text 替换当前选中的内容。
pub fn replace_selection(text: &str) {
    let Ok(mut cb) = Clipboard::new() else { return };
    let saved = cb.get_text().ok();
    if cb.set_text(text).is_err() {
        return;
    }
    chord(keys::V);
    // 目标程序读剪贴板是异步的，等一下再恢复
    sleep(Duration::from_millis(150));
    restore(&mut cb, saved);
}

pub fn copy(text: &str) {
    if let Ok(mut cb) = Clipboard::new() {
        let _ = cb.set_text(text);
    }
}
