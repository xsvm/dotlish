//! 云端服务。第一版只用免 key 的接口，见 服务调研.md。

pub mod alibaba;
pub mod bing;
pub mod google;
pub mod languagetool;
pub mod mymemory;
pub mod ovh;
pub mod volcengine;
pub mod youdao;

use crate::config::Config;
use serde_json::Value;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
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

pub fn post_json(agent: &Agent, url: &str, body: &Value) -> Res<String> {
    read(
        agent
            .post(url)
            .header("Content-Type", "application/json")
            .send(body.to_string()),
    )
}

/// 网页接口的令牌缓存：令牌能用很久，缓存后每次翻译只发一个请求。
pub struct TokenCache<T>(Mutex<Option<(T, Instant)>>);

impl<T: Clone> TokenCache<T> {
    pub const fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// `call` 返回 `Ok(None)` 表示令牌被拒绝。缓存过期或令牌被拒绝时重新取一次令牌再试；
    /// 网络错误直接返回，不重试。
    pub fn call<R>(
        &self,
        ttl: Duration,
        fetch: impl Fn() -> Res<T>,
        call: impl Fn(&T) -> Res<Option<R>>,
    ) -> Res<R> {
        let cached = self
            .lock()
            .as_ref()
            .filter(|(_, at)| at.elapsed() < ttl)
            .map(|(t, _)| t.clone());
        if let Some(t) = cached
            && let Some(r) = call(&t)?
        {
            return Ok(r);
        }
        let t = fetch()?;
        *self.lock() = Some((t.clone(), Instant::now()));
        call(&t)?.ok_or_else(|| "令牌被拒绝".into())
    }

    fn lock(&self) -> MutexGuard<'_, Option<(T, Instant)>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
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
}

type TranslateFn = fn(&Agent, &str, &str) -> Res<String>;

/// 按顺序尝试，前一个失败自动换下一个。加新服务只需要在这里加一行。
const TRANSLATORS: &[(&str, TranslateFn)] = &[
    ("Bing", bing::translate),
    ("火山", volcengine::translate),
    ("阿里", alibaba::translate),
    ("Google", google::translate),
    ("MyMemory", mymemory::translate),
];

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<Translation> {
    let target = if contains_cjk(text) { "en" } else { target };
    let mut errors = Vec::new();
    for (name, f) in TRANSLATORS {
        match f(agent, text, target) {
            Ok(t) => {
                return Ok(Translation {
                    text: t,
                    service: name,
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
    fn token_cache() {
        use std::cell::Cell;
        let cache = TokenCache::new();
        let fetches = Cell::new(0);
        let fetch = || {
            fetches.set(fetches.get() + 1);
            Ok(fetches.get())
        };
        let ttl = Duration::from_secs(60);
        // 第一次取令牌，第二次复用
        assert_eq!(cache.call(ttl, fetch, |&t| Ok(Some(t))), Ok(1));
        assert_eq!(cache.call(ttl, fetch, |&t| Ok(Some(t))), Ok(1));
        // 旧令牌被拒绝：重取一次
        assert_eq!(cache.call(ttl, fetch, |&t| Ok((t > 1).then_some(t))), Ok(2));
        // 新令牌也被拒绝：报错，不再循环
        assert!(cache.call(ttl, fetch, |_| Ok(None::<i32>)).is_err());
        assert_eq!(fetches.get(), 3);
        // 网络错误不重试
        assert!(
            cache
                .call(ttl, fetch, |_| Err::<Option<i32>, _>("断网".into()))
                .is_err()
        );
        assert_eq!(fetches.get(), 3);
        // 过期后重取
        assert_eq!(cache.call(Duration::ZERO, fetch, |&t| Ok(Some(t))), Ok(4));
    }

    #[test]
    fn cjk() {
        assert!(contains_cjk("hello 世界"));
        assert!(!contains_cjk("hello world"));
    }
}
