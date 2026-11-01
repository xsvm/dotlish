//! 弹窗：按快捷键后取词、请求服务、显示结果、复制或替换回原处。

use crate::hotkeys::{self, Action};
use crate::services::languagetool::{self, Issue};
use crate::services::ovh;
use crate::services::youdao::{self, Accent, Entry};
use crate::services::{self, Res, Translation};
use crate::{DictData, IssueItem, Popup, app, audio, input, popup_size};
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel, Weak};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// 当前这次操作的数据；只在 UI 线程访问。
#[derive(Default)]
struct State {
    /// 每次发起请求或关闭弹窗加一；请求结果回来时序号对不上就丢弃
    seq: u64,
    action: Option<Action>,
    text: String,
    issues: Vec<Issue>,
    picks: Vec<Option<usize>>,
    /// 纠错列表的模型；换候选时只更新对应的一行
    model: Rc<VecModel<IssueItem>>,
    /// 复制/替换时使用的结果
    result: String,
    /// 翻译页右侧正在显示的单词
    word: String,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
    /// 拖动选择时选区连续变化，停下来再查词
    static WORD_TIMER: Timer = Timer::default();
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

    let weak = ui.as_weak();
    ui.on_selection_changed(move |pane, anchor, cursor| {
        let weak = weak.clone();
        WORD_TIMER.with(|t| {
            t.start(
                TimerMode::SingleShot,
                Duration::from_millis(150),
                move || {
                    if let Some(ui) = weak.upgrade() {
                        pick_word(&ui, pane, anchor, cursor);
                    }
                },
            )
        });
    });

    ui.on_speak(|word, accent| {
        let accent = if accent == 1 { Accent::Uk } else { Accent::Us };
        audio::pronounce(app::with(|a| a.agent.clone()), word.into(), accent);
    });

    let weak = ui.as_weak();
    ui.on_close_word(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_word_open(false);
        }
        STATE.with(|s| s.borrow_mut().word.clear());
    });
}

/// 点在单词上：选中整个词；选中的正好是一个词：直接用。然后在右侧查释义。
fn pick_word(ui: &Popup, pane: i32, anchor: i32, cursor: i32) {
    let (a, c) = (anchor.max(0) as usize, cursor.max(0) as usize);
    let found = STATE.with(|s| {
        let s = s.borrow();
        let text = if pane == 0 { &s.text } else { &s.result };
        let (start, end) = word_range(text, a.min(c), a.max(c))?;
        let word = &text[start..end];
        (!word.eq_ignore_ascii_case(&s.word)).then(|| (word.to_string(), start, end))
    });
    let Some((word, start, end)) = found else {
        return;
    };
    if a == c {
        ui.invoke_select_word(pane, start as i32, end as i32);
    }

    let seq = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.word = word.clone();
        s.seq
    });
    ui.set_word_dict(DictData {
        word: word.as_str().into(),
        ..Default::default()
    });
    ui.set_word_note("查询中…".into());
    ui.set_word_open(true);

    let agent = app::with(|a| a.agent.clone());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let r = youdao::lookup(&agent, &word);
        deliver(weak, seq, move |ui| {
            // 查询期间又点了别的词
            if STATE.with(|s| s.borrow().word != word) {
                return;
            }
            match r {
                Ok(e) => {
                    ui.set_word_dict(dict_data(&e));
                    ui.set_word_note("".into());
                }
                Err(e) => ui.set_word_note(e.into()),
            }
        });
    });
}

/// 选区 [start, end)（UTF-8 字节偏移）对应的英文单词范围。
/// start == end 时取光标所在的词；否则选中的内容去掉两端标点、空格后必须正好是一个词。
/// 只扫描这个词附近，O(词长)。
fn word_range(text: &str, start: usize, end: usize) -> Option<(usize, usize)> {
    let b = text.as_bytes();
    let is_word = |c: &&u8| c.is_ascii_alphabetic() || **c == b'-' || **c == b'\'';
    let (s, e) = if start == end {
        let p = start.min(b.len());
        (
            p - b[..p].iter().rev().take_while(is_word).count(),
            p + b[p..].iter().take_while(is_word).count(),
        )
    } else {
        (start, end)
    };
    // 只去掉两端的标点和空格；中文等文字不去掉，这样选中中英混排时不会误判成一个词
    let not_letter = |c: char| !c.is_alphabetic();
    let range = text.get(s..e)?;
    let s = s + range.len() - range.trim_start_matches(not_letter).len();
    let w = range.trim_matches(not_letter);
    services::is_single_word(w).then_some((s, s + w.len()))
}

fn dict_data(e: &Entry) -> DictData {
    DictData {
        word: e.word.as_str().into(),
        uk: e.uk.as_str().into(),
        us: e.us.as_str().into(),
        senses: ModelRc::new(
            e.senses
                .iter()
                .map(SharedString::from)
                .collect::<VecModel<_>>(),
        ),
        forms: e.forms.as_str().into(),
    }
}

fn hide(weak: &Weak<Popup>) {
    if let Some(ui) = weak.upgrade() {
        let _ = ui.hide();
    }
    hotkeys::watch_clicks(false);
    // 还没回来的结果作废，免得关掉后又弹出来
    STATE.with(|s| s.borrow_mut().seq += 1);
}

/// 鼠标在弹窗外按下时关闭弹窗，和其他划词软件一样。
pub fn on_click() {
    let ui = app::popup();
    let window = ui.window();
    if !window.is_visible() {
        return;
    }
    let Some((x, y)) = input::cursor() else {
        return;
    };
    // 光标坐标：Windows 是物理像素，macOS 是逻辑点（同 show_at_cursor）
    let unit = if cfg!(target_os = "macos") {
        window.scale_factor()
    } else {
        1.0
    };
    let (pos, size) = (window.position(), window.size());
    let rect = popup_size::Rect {
        x: pos.x as f32 / unit,
        y: pos.y as f32 / unit,
        w: size.width as f32 / unit,
        h: size.height as f32 / unit,
    };
    if !rect.contains((x as f32, y as f32)) {
        hide(&ui.as_weak());
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
    // 弹窗像输入法候选框一样不进任务栏
    #[cfg(windows)]
    {
        use slint::winit_030::WinitWindowAccessor;
        use slint::winit_030::winit::platform::windows::WindowExtWindows;
        ui.window().with_winit_window(|w| w.set_skip_taskbar(true));
    }
    hotkeys::watch_clicks(true);
}

pub fn run(action: Action) {
    let ui = app::popup();
    let visible = ui.window().is_visible();
    let text = if visible {
        // 弹窗开着时按另一个快捷键：沿用这次的文字切换功能（按错键时不用关掉重来）。
        // 这时焦点在弹窗上，再模拟复制也取不到原来的选中内容。
        let (text, current) = STATE.with(|s| {
            let s = s.borrow();
            (s.text.clone(), s.action)
        });
        if current == Some(action) {
            return;
        }
        text
    } else {
        let Some(text) = input::grab_selection() else {
            return;
        };
        text
    };
    let (agent, target) = app::with(|a| (a.agent.clone(), a.cfg.target_lang.clone()));
    let seq = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let seq = s.seq + 1;
        *s = State {
            seq,
            action: Some(action),
            text: text.clone(),
            ..Default::default()
        };
        seq
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
    ui.set_word_open(false);
    ui.invoke_reset_focus();
    // 切换功能时弹窗留在原处
    if !visible {
        show_at_cursor(&ui);
    }

    let weak = ui.as_weak();
    std::thread::spawn(move || match action {
        Action::Check => {
            // 只用 AI；所有模型都失败或在限流冷却时才退回 LanguageTool
            let r = ovh::check(&agent, &text).or_else(|ai| {
                languagetool::check(&agent, &text)
                    .map(|i| (i, "LanguageTool（AI 暂不可用）"))
                    .map_err(|lt| format!("{ai}\nLanguageTool：{lt}"))
            });
            deliver(weak, seq, move |ui| on_checked(ui, r));
        }
        Action::Translate if services::is_single_word(&text) => {
            let r = youdao::lookup(&agent, text.trim());
            deliver(weak, seq, move |ui| on_looked_up(ui, r));
        }
        Action::Translate => {
            let r = services::translate(&agent, &text, &target);
            deliver(weak, seq, move |ui| on_translated(ui, &text, r));
        }
    });
}

/// 把结果交给 UI 线程；用户已经关掉弹窗或切换了功能时丢弃。
fn deliver(weak: Weak<Popup>, seq: u64, f: impl FnOnce(&Popup) + Send + 'static) {
    let _ = weak.upgrade_in_event_loop(move |ui| {
        if STATE.with(|s| s.borrow().seq) == seq {
            f(&ui);
        }
    });
}

fn on_checked(ui: &Popup, r: Res<(Vec<Issue>, &'static str)>) {
    let (issues, service) = match r {
        Ok(r) => r,
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
        ui.set_service(service.into());
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
    ui.set_dict(dict_data(&entry));
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

#[cfg(test)]
mod tests {
    use super::word_range;

    fn word(text: &str, start: usize, end: usize) -> Option<&str> {
        word_range(text, start, end).map(|(s, e)| &text[s..e])
    }

    #[test]
    fn click_inside_word() {
        let t = "The popup holds keyboard focus.";
        assert_eq!(word(t, 6, 6), Some("popup"));
        // 光标在词首、词尾都算这个词
        assert_eq!(word(t, 4, 4), Some("popup"));
        assert_eq!(word(t, 30, 30), Some("focus"));
        assert_eq!(word(t, 31, 31), None);
    }

    #[test]
    fn click_keeps_apostrophe_and_hyphen() {
        assert_eq!(word("it's well-known", 1, 1), Some("it's"));
        assert_eq!(word("it's well-known", 7, 7), Some("well-known"));
        assert_eq!(word("say 'hello'", 7, 7), Some("hello"));
    }

    #[test]
    fn selection_must_be_one_word() {
        let t = "The popup holds";
        assert_eq!(word(t, 3, 10), Some("popup"));
        assert_eq!(word(t, 0, 9), None);
    }

    #[test]
    fn non_english() {
        let t = "弹出窗口 focus 焦点";
        assert_eq!(word(t, 3, 3), None);
        assert_eq!(word(t, 0, t.len()), None);
        assert_eq!(word(t, 12, 19), Some("focus"));
        assert_eq!(word(t, 12, 25), None);
    }
}
