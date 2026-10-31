//! 设置窗口：从托盘进入。编辑的是一份草稿，点保存才生效。

use crate::config::{self, Config, Hotkeys};
use crate::hotkeys::{self, ACTIONS, Mode, binding};
use crate::{HotkeyRow, SettingsWindow, app, services, tray};
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

fn row_text(spec: &str, conflict: bool) -> String {
    if spec.is_empty() {
        return "未设置".into();
    }
    let b = match binding::parse(spec) {
        Ok(b) => b,
        Err(_) => return format!("{spec}（无法识别）"),
    };
    let mut text = binding::display(b);
    if conflict {
        text.push_str("（冲突）");
    } else if let Some(w) = b.warning() {
        text.push_str(&format!("（{w}）"));
    }
    text
}

fn refresh_rows(ui: &SettingsWindow) {
    DRAFT.with(|d| {
        let d = d.borrow();
        let conflicts = hotkeys::parse_all(&d)
            .map(|b| hotkeys::conflicts(&b))
            .unwrap_or_default();
        let rows: VecModel<HotkeyRow> = ACTIONS
            .iter()
            .map(|(action, name)| HotkeyRow {
                name: (*name).into(),
                display: row_text(hotkeys::get(&d, *action), conflicts.contains(action)).into(),
            })
            .collect();
        ui.set_hotkeys(ModelRc::new(rows));
    });
}

fn set_draft(ui: &SettingsWindow, i: usize, spec: String) {
    DRAFT.with(|d| hotkeys::set(&mut d.borrow_mut(), ACTIONS[i].0, spec));
    stop_recording(ui);
    refresh_rows(ui);
}

fn stop_recording(ui: &SettingsWindow) {
    ui.set_recording(-1);
    hotkeys::set_mode(Mode::Paused);
}

fn fill(ui: &SettingsWindow, cfg: &Config) {
    DRAFT.with(|d| *d.borrow_mut() = cfg.hotkeys.clone());
    ui.set_proxy(cfg.proxy.as_str().into());
    ui.set_language_index(language_index(&cfg.target_lang));
    stop_recording(ui);
    refresh_rows(ui);
}

pub fn open(error: &str) {
    let ui = app::settings();
    let cfg = app::with(|a| a.cfg.clone());
    // 设置窗口打开期间不触发快捷键
    fill(&ui, &cfg);
    ui.set_error(error.into());
    let _ = ui.show();
    tray::apply_window_icon(ui.window());
}

/// 钩子录到按键（None 表示用户按 Esc 取消）。
pub fn on_recorded(recorded: Option<binding::Binding>) {
    let ui = app::settings();
    let Ok(i) = usize::try_from(ui.get_recording()) else {
        return;
    };
    match recorded {
        Some(b) => set_draft(&ui, i, binding::format(b)),
        None => stop_recording(&ui),
    }
}

/// 按配置启用快捷键。
pub fn activate(cfg: &Config) -> Result<(), String> {
    let bindings = hotkeys::parse_all(&cfg.hotkeys).map_err(|e| e.join("\n"))?;
    hotkeys::set_bindings(bindings);
    hotkeys::set_mode(Mode::Normal);
    Ok(())
}

/// 关闭但不保存：恢复原来的快捷键。
fn close(ui: &SettingsWindow) {
    let cfg = app::with(|a| a.cfg.clone());
    match activate(&cfg) {
        Ok(()) => {
            let _ = ui.hide();
        }
        Err(e) => ui.set_error(e.into()),
    }
}

fn save(ui: &SettingsWindow) -> Result<(), String> {
    let hotkeys = DRAFT.with(|d| d.borrow().clone());
    let bindings = hotkeys::parse_all(&hotkeys).map_err(|e| e.join("\n"))?;
    if !hotkeys::conflicts(&bindings).is_empty() {
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
    config::save(&cfg)?;
    activate(&cfg)?;
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
    ui.on_toggle_record(move |i| {
        let Some(ui) = weak.upgrade() else { return };
        if ui.get_recording() == i {
            stop_recording(&ui);
        } else {
            ui.set_recording(i);
            hotkeys::set_mode(Mode::Recording);
        }
    });

    let weak = ui.as_weak();
    ui.on_clear_hotkey(move |i| {
        if let Some(ui) = weak.upgrade() {
            set_draft(&ui, i as usize, String::new());
        }
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
