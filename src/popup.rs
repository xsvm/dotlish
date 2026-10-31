//! 弹窗：按快捷键后取词、请求服务、显示结果、复制或替换回原处。

use crate::hotkeys::Action;
use crate::services::languagetool::{self, Issue};
use crate::services::youdao::{self, Entry};
use crate::services::{self, Res, Translation};
use crate::{IssueItem, Popup, app, input, popup_size};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel, Weak};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

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

/// 绑定弹窗的回调，启动时调用一次。
pub fn setup(ui: &Popup) {
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

pub fn run(action: Action) {
    let Some(text) = input::grab_selection() else {
        return;
    };
    let ui = app::popup();
    let (agent, target) = app::with(|a| (a.agent.clone(), a.cfg.target_lang.clone()));
    STATE.with(|s| {
        *s.borrow_mut() = State {
            text: text.clone(),
            ..Default::default()
        }
    });

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
    show_at_cursor(&ui);

    let weak = ui.as_weak();
    std::thread::spawn(move || match action {
        Action::Check => {
            let r = languagetool::check(&agent, &text);
            let _ = weak.upgrade_in_event_loop(move |ui| on_checked(&ui, r));
        }
        Action::Translate if services::is_single_word(&text) => {
            let r = youdao::lookup(&agent, text.trim());
            let _ = weak.upgrade_in_event_loop(move |ui| on_looked_up(&ui, r));
        }
        Action::Translate => {
            let r = services::translate(&agent, &text, &target);
            let _ = weak.upgrade_in_event_loop(move |ui| on_translated(&ui, &text, r));
        }
    });
}

fn on_checked(ui: &Popup, r: Res<Vec<Issue>>) {
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

fn on_looked_up(ui: &Popup, r: Res<Entry>) {
    let entry = match r {
        Ok(e) => e,
        Err(e) => return show_error(ui, &e),
    };
    ui.set_mode("dict".into());
    ui.set_heading("词典".into());
    ui.set_service("有道词典".into());
    ui.set_source_text(entry.word.as_str().into());
    ui.set_phonetic(entry.phonetic.as_str().into());
    ui.set_senses(ModelRc::new(
        entry
            .senses
            .iter()
            .map(SharedString::from)
            .collect::<VecModel<_>>(),
    ));
    ui.set_extra(entry.forms.as_str().into());
    STATE.with(|s| s.borrow_mut().result = entry.to_text());
}

fn show_error(ui: &Popup, msg: &str) {
    ui.set_mode("error".into());
    ui.set_note("出错了".into());
    ui.set_result_text(msg.into());
    if !ui.window().is_visible() {
        show_at_cursor(ui);
    }
}
