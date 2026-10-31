//! 设置窗口：从托盘进入。编辑的是一份草稿，点保存才生效。

use crate::config::{self, Config, Hotkeys};
use crate::hotkeys::{self, ACTIONS, Mods};
use crate::{HotkeyRow, SettingsWindow, app, services};
use slint::{CloseRequestResponse, ComponentHandle, ModelRc, SharedString, VecModel};
use std::cell::RefCell;

/// (语言代码, 显示名)
const LANGUAGES: [(&str, &str); 7] = [
    ("zh-CN", "简体中文"),
    ("en", "English"),
    ("ja", "日本語"),
    ("ko", "한국어"),
    ("fr", "Français"),
    ("de", "Deutsch"),
    ("es", "Español"),
];

thread_local! {
    static DRAFT: RefCell<Hotkeys> = RefCell::default();
}

fn language_index(code: &str) -> i32 {
    LANGUAGES.iter().position(|(c, _)| *c == code).unwrap_or(0) as i32
}

fn refresh_rows(ui: &SettingsWindow) {
    DRAFT.with(|d| {
        let d = d.borrow();
        let conflicts = hotkeys::conflicts(&d);
        let rows: VecModel<HotkeyRow> = ACTIONS
            .iter()
            .map(|(action, name)| {
                let mut display = hotkeys::display(hotkeys::get(&d, *action));
                if conflicts.contains(action) {
                    display.push_str("（冲突）");
                }
                HotkeyRow {
                    name: (*name).into(),
                    display: display.into(),
                }
            })
            .collect();
        ui.set_hotkeys(ModelRc::new(rows));
    });
}

fn fill(ui: &SettingsWindow, cfg: &Config) {
    DRAFT.with(|d| *d.borrow_mut() = cfg.hotkeys.clone());
    ui.set_proxy(cfg.proxy.as_str().into());
    ui.set_language_index(language_index(&cfg.target_lang));
    ui.set_recording(-1);
    refresh_rows(ui);
}

pub fn open(error: &str) {
    let ui = app::settings();
    let cfg = app::with(|a| {
        // 录制时按键不能被全局快捷键拦走
        a.registry.clear();
        a.cfg.clone()
    });
    fill(&ui, &cfg);
    ui.set_error(error.into());
    let _ = ui.show();
}

/// 关闭但不保存：恢复原来的快捷键。
fn close(ui: &SettingsWindow) {
    let errors = app::with(|a| {
        let cfg = a.cfg.clone();
        a.registry.apply(&cfg.hotkeys)
    });
    if errors.is_empty() {
        let _ = ui.hide();
    } else {
        ui.set_error(errors.join("\n").into());
    }
}

fn save(ui: &SettingsWindow) -> Result<(), String> {
    let hotkeys = DRAFT.with(|d| d.borrow().clone());
    if !hotkeys::conflicts(&hotkeys).is_empty() {
        return Err("有快捷键冲突，请修改标出“冲突”的项。".into());
    }
    let cfg = Config {
        hotkeys,
        proxy: ui.get_proxy().trim().to_string(),
        target_lang: LANGUAGES[ui.get_language_index().max(0) as usize]
            .0
            .to_string(),
    };
    let agent = services::agent(&cfg)?;
    let errors = app::with(|a| {
        let errors = a.registry.apply(&cfg.hotkeys);
        if !errors.is_empty() {
            a.registry.clear();
        }
        errors
    });
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    config::save(&cfg)?;
    app::with(|a| {
        a.cfg = cfg;
        a.agent = agent;
    });
    Ok(())
}

/// 绑定设置窗口的回调，启动时调用一次。
pub fn setup(ui: &SettingsWindow) {
    ui.set_languages(ModelRc::new(
        LANGUAGES
            .iter()
            .map(|(_, n)| SharedString::from(*n))
            .collect::<VecModel<_>>(),
    ));

    let weak = ui.as_weak();
    ui.on_key_recorded(move |i, text, control, alt, shift, meta| {
        let Some(ui) = weak.upgrade() else { return };
        let Some(spec) = hotkeys::record(
            &text,
            Mods {
                control,
                alt,
                shift,
                meta,
            },
        ) else {
            // 还没按完整，继续录制
            return;
        };
        DRAFT.with(|d| hotkeys::set(&mut d.borrow_mut(), ACTIONS[i as usize].0, spec));
        ui.set_recording(-1);
        refresh_rows(&ui);
    });

    let weak = ui.as_weak();
    ui.on_clear_hotkey(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        DRAFT.with(|d| hotkeys::set(&mut d.borrow_mut(), ACTIONS[i as usize].0, String::new()));
        ui.set_recording(-1);
        refresh_rows(&ui);
    });

    let weak = ui.as_weak();
    ui.on_reset_defaults(move || {
        if let Some(ui) = weak.upgrade() {
            fill(&ui, &Config::default());
        }
    });

    let weak = ui.as_weak();
    ui.on_cancel(move || {
        if let Some(ui) = weak.upgrade() {
            close(&ui);
        }
    });

    let weak = ui.as_weak();
    ui.on_save(move || {
        let Some(ui) = weak.upgrade() else { return };
        match save(&ui) {
            Ok(()) => {
                let _ = ui.hide();
            }
            Err(e) => ui.set_error(e.into()),
        }
    });

    let weak = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = weak.upgrade() {
            close(&ui);
        }
        // close 里已经决定是否隐藏
        CloseRequestResponse::KeepWindowShown
    });
}
