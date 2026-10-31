#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod input;
mod popup_size;
mod services;

use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use services::languagetool::{self, Issue};
use services::{Res, Translation, youdao};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;
use ureq::Agent;

slint::include_modules!();

#[derive(Clone, Copy, PartialEq)]
enum Action {
    Check,
    Fix,
    Translate,
}

/// 当前这次操作的数据；只在 UI 线程访问。
#[derive(Default)]
struct State {
    text: String,
    issues: Vec<Issue>,
    picks: Vec<Option<usize>>,
    /// 纠错列表的模型；换候选时只更新对应的一行
    model: Rc<VecModel<IssueItem>>,
    /// 复制/替换时使用的结果
    result: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = config::load();
    let agent = services::agent(&cfg)?;
    let ui = Popup::new()?;

    let manager = GlobalHotKeyManager::new()?;
    let mut actions = Vec::new();
    for (spec, action) in [
        (&cfg.hotkeys.check, Action::Check),
        (&cfg.hotkeys.fix, Action::Fix),
        (&cfg.hotkeys.translate, Action::Translate),
    ] {
        let hotkey: HotKey = spec
            .parse()
            .map_err(|e| format!("快捷键 {spec} 无效：{e}"))?;
        let id = hotkey.id();
        manager
            .register(hotkey)
            .map_err(|e| format!("快捷键 {spec} 注册失败，可能被其他程序占用：{e}"))?;
        actions.push((id, action));
    }

    let hotkey_timer = slint::Timer::default();
    {
        let weak = ui.as_weak();
        let target = cfg.target_lang.clone();
        hotkey_timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(30),
            move || {
                while let Ok(ev) = GlobalHotKeyEvent::receiver().try_recv() {
                    if ev.state != HotKeyState::Pressed {
                        continue;
                    }
                    let action = actions.iter().find(|(id, _)| *id == ev.id).map(|(_, a)| *a);
                    if let (Some(action), Some(ui)) = (action, weak.upgrade()) {
                        on_hotkey(&ui, &agent, &target, action);
                    }
                }
            },
        );
    }

    let weak = ui.as_weak();
    ui.on_dismiss(move || hide(&weak));

    let weak = ui.as_weak();
    ui.on_copy_result(move || {
        STATE.with(|s| input::copy(&s.borrow().result));
        hide(&weak);
    });

    let weak = ui.as_weak();
    ui.on_replace(move || {
        let text = STATE.with(|s| s.borrow().result.clone());
        hide(&weak);
        // 等焦点回到原来的窗口再粘贴
        slint::Timer::single_shot(Duration::from_millis(120), move || {
            input::replace_selection(&text)
        });
    });

    let weak = ui.as_weak();
    ui.on_pick(move |i, j| {
        let Some(ui) = weak.upgrade() else { return };
        let i = i as usize;
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            let Some(p) = s.picks.get_mut(i) else { return };
            *p = usize::try_from(j).ok();
            s.result = languagetool::apply(&s.text, &s.issues, &s.picks);
            s.model.set_row_data(i, issue_item(&s, i));
            show_check_summary(&ui, &s);
        });
    });

    slint::run_event_loop_until_quit()?;
    drop(manager);
    Ok(())
}

fn hide(weak: &Weak<Popup>) {
    if let Some(ui) = weak.upgrade() {
        let _ = ui.hide();
    }
}

fn show_at_cursor(ui: &Popup) {
    if let Some((x, y)) = input::cursor()
        && let Ok(d) = display_info::DisplayInfo::from_point(x, y)
    {
        // display-info 在 Windows 上给物理像素，在 macOS 上给逻辑点
        let unit = if cfg!(target_os = "macos") {
            1.0
        } else {
            d.scale_factor.max(1.0)
        };
        let screen = popup_size::Rect {
            x: d.x as f32,
            y: d.y as f32,
            w: d.width as f32,
            h: d.height as f32,
        };
        let p = popup_size::place((x as f32, y as f32), screen, unit);
        let window = ui.window();
        window.set_size(slint::LogicalSize::new(p.size.0, p.size.1));
        if cfg!(target_os = "macos") {
            window.set_position(slint::LogicalPosition::new(p.pos.0, p.pos.1));
        } else {
            window.set_position(slint::PhysicalPosition::new(p.pos.0 as i32, p.pos.1 as i32));
        }
    }
    let _ = ui.show();
}

fn on_hotkey(ui: &Popup, agent: &Agent, target: &str, action: Action) {
    let Some(text) = input::grab_selection() else {
        return;
    };
    STATE.with(|s| {
        *s.borrow_mut() = State {
            text: text.clone(),
            ..Default::default()
        }
    });

    if action != Action::Fix {
        ui.set_mode("loading".into());
        ui.set_heading(
            if action == Action::Check {
                "纠错"
            } else {
                "翻译"
            }
            .into(),
        );
        ui.set_note("".into());
        ui.set_service("".into());
        show_at_cursor(ui);
    }

    let weak = ui.as_weak();
    let agent = agent.clone();
    let target = target.to_string();
    std::thread::spawn(move || match action {
        Action::Check | Action::Fix => {
            let r = languagetool::check(&agent, &text);
            let _ = weak.upgrade_in_event_loop(move |ui| on_checked(&ui, action, r));
        }
        Action::Translate if services::is_single_word(&text) => {
            let word = text.trim().to_string();
            let r = youdao::lookup(&agent, &word);
            let _ = weak.upgrade_in_event_loop(move |ui| on_looked_up(&ui, word, r));
        }
        Action::Translate => {
            let r = services::translate(&agent, &text, &target);
            let _ = weak.upgrade_in_event_loop(move |ui| on_translated(&ui, &text, r));
        }
    });
}

fn on_checked(ui: &Popup, action: Action, r: Res<Vec<Issue>>) {
    let issues = match r {
        Ok(i) => i,
        Err(e) => return show_error(ui, &e),
    };
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.picks = issues
            .iter()
            .map(|i| (!i.replacements.is_empty()).then_some(0))
            .collect();
        s.issues = issues;
        s.result = languagetool::apply(&s.text, &s.issues, &s.picks);

        if action == Action::Fix {
            // 一键修正不弹窗，焦点还在原程序里，直接粘贴
            if !s.issues.is_empty() {
                input::replace_selection(&s.result);
            }
            return;
        }
        s.model = Rc::new((0..s.issues.len()).map(|i| issue_item(&s, i)).collect());
        ui.set_issues(ModelRc::from(s.model.clone()));
        ui.set_mode("check".into());
        ui.set_service("LanguageTool".into());
        show_check_summary(ui, &s);
    });
}

fn issue_item(s: &State, i: usize) -> IssueItem {
    let issue = &s.issues[i];
    IssueItem {
        message: issue.message.as_str().into(),
        original: issue.original(&s.text).into(),
        candidates: ModelRc::new(
            issue
                .replacements
                .iter()
                .map(SharedString::from)
                .collect::<VecModel<_>>(),
        ),
        picked: s.picks[i].map_or(-1, |j| j as i32),
    }
}

fn show_check_summary(ui: &Popup, s: &State) {
    let changed = s.picks.iter().filter(|p| p.is_some()).count();
    ui.set_note(if s.issues.is_empty() {
        "没有发现问题".into()
    } else {
        format!("{changed}/{} 处修改", s.issues.len()).into()
    });
    ui.set_result_text(s.result.as_str().into());
}

fn on_translated(ui: &Popup, source: &str, r: Res<Translation>) {
    let t = match r {
        Ok(t) => t,
        Err(e) => return show_error(ui, &e),
    };
    ui.set_mode("translate".into());
    ui.set_service(t.service.into());
    ui.set_note(t.note.into());
    ui.set_source_text(source.into());
    ui.set_result_text(t.text.as_str().into());
    STATE.with(|s| s.borrow_mut().result = t.text);
}

fn on_looked_up(ui: &Popup, word: String, r: Res<String>) {
    let explain = match r {
        Ok(e) => e,
        Err(e) => return show_error(ui, &e),
    };
    ui.set_mode("dict".into());
    ui.set_heading("词典".into());
    ui.set_service("有道词典".into());
    ui.set_source_text(word.into());
    ui.set_result_text(explain.as_str().into());
    STATE.with(|s| s.borrow_mut().result = explain);
}

fn show_error(ui: &Popup, msg: &str) {
    ui.set_mode("error".into());
    ui.set_note("出错了".into());
    ui.set_result_text(msg.into());
    if !ui.window().is_visible() {
        show_at_cursor(ui);
    }
}
