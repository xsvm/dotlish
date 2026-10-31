//! 云端服务。第一版只用免 key 的接口，见 服务调研.md。

pub mod google;
pub mod languagetool;
pub mod mymemory;
pub mod youdao;

use crate::config::Config;
use serde_json::Value;
use std::time::Duration;
use ureq::{Agent, Body, Proxy, http::Response};

pub type Res<T> = Result<T, String>;

/// 整个程序共用一个 Agent（内部是 Arc，clone 很便宜），复用连接，省掉每次的 TLS 握手。
pub fn agent(cfg: &Config) -> Res<Agent> {
    let mut builder = Agent::config_builder().timeout_global(Some(Duration::from_secs(8)));
    if !cfg.proxy.is_empty() {
        let proxy = Proxy::new(&cfg.proxy).map_err(|e| format!("代理地址无效：{e}"))?;
        builder = builder.proxy(Some(proxy));
    }
    Ok(builder.build().into())
}

fn read(resp: Result<Response<Body>, ureq::Error>) -> Res<String> {
    resp.map_err(|e| e.to_string())?
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())
}

pub fn get(agent: &Agent, url: &str, query: &[(&str, &str)]) -> Res<String> {
    let mut req = agent.get(url);
    for (k, v) in query {
        req = req.query(*k, *v);
    }
    read(req.call())
}

pub fn post_form(agent: &Agent, url: &str, form: &[(&str, &str)]) -> Res<String> {
    read(agent.post(url).send_form(form.iter().copied()))
}

pub fn json(service: &str, body: &str) -> Res<Value> {
    serde_json::from_str(body).map_err(|e| format!("{service} 返回格式异常：{e}"))
}

pub fn contains_cjk(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}'))
}

/// 选中的是单个英文词时走词典。
pub fn is_single_word(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty()
        && s.len() <= 40
        && s.chars()
            .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '\'')
}

pub struct Translation {
    pub text: String,
    pub service: &'static str,
    pub note: String,
}

type TranslateFn = fn(&Agent, &str, &str) -> Res<String>;

/// 按顺序尝试，前一个失败自动换下一个。加新服务只需要在这里加一行。
const TRANSLATORS: &[(&str, TranslateFn)] = &[
    ("Google", google::translate),
    ("MyMemory", mymemory::translate),
];

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<Translation> {
    let target = if contains_cjk(text) { "en" } else { target };
    let mut errors = Vec::new();
    for (i, (name, f)) in TRANSLATORS.iter().enumerate() {
        match f(agent, text, target) {
            Ok(t) => {
                let note = if i == 0 {
                    String::new()
                } else {
                    format!("{} 不可用，已改用 {name}", TRANSLATORS[0].0)
                };
                return Ok(Translation {
                    text: t,
                    service: name,
                    note,
                });
            }
            Err(e) => errors.push(format!("{name}：{e}")),
        }
    }
    Err(errors.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_word() {
        assert!(is_single_word(" serendipity "));
        assert!(is_single_word("don't"));
        assert!(!is_single_word("two words"));
        assert!(!is_single_word("中文"));
        assert!(!is_single_word(""));
    }

    #[test]
    fn cjk() {
        assert!(contains_cjk("hello 世界"));
        assert!(!contains_cjk("hello world"));
    }
}
