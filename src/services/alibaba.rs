//! 阿里翻译网页接口（translate.alibaba.com）。不需要 key，国内直连；非官方，可能变动。
//! 先取 csrf 令牌再提交表单；令牌可复用，缓存起来。

use super::{Res, TokenCache, get, json, post_form};
use std::time::Duration;
use ureq::Agent;

const CSRF_URL: &str = "https://translate.alibaba.com/api/translate/csrftoken";
const API: &str = "https://translate.alibaba.com/api/translate/text";
/// 有效期未公开；过期时接口返回网页，会自动重取。
const TTL: Duration = Duration::from_secs(30 * 60);

static CSRF: TokenCache<String> = TokenCache::new();

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<String> {
    let to = lang(target);
    CSRF.call(
        TTL,
        || {
            json("阿里", &get(agent, CSRF_URL, &[])?)?["token"]
                .as_str()
                .map(String::from)
                .ok_or_else(|| "阿里没有返回 csrf 令牌".into())
        },
        |token| {
            let form = [
                ("srcLang", "auto"),
                ("tgtLang", to),
                ("domain", "general"),
                ("query", text),
                ("_csrf", token.as_str()),
            ];
            parse(&post_form(agent, API, &form)?)
        },
    )
}

fn lang(target: &str) -> &str {
    if target == "zh-CN" { "zh" } else { target }
}

/// 令牌无效时返回的是网页而不是 JSON。
fn parse(body: &str) -> Res<Option<String>> {
    if !body.trim_start().starts_with('{') {
        return Ok(None);
    }
    let v = json("阿里", body)?;
    if v["success"].as_bool() != Some(true) {
        return Err(v["message"].as_str().unwrap_or("未知错误").to_string());
    }
    v["data"]["translateText"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| Some(unescape_html(s)))
        .ok_or_else(|| "阿里返回中没有译文".into())
}

/// 阿里的译文带 HTML 实体（`&#39;`、`&lt;`）。一次遍历，O(n)。
fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // 实体最长也就十来个字节，只在这个范围里找分号，避免长文本退化成 O(n²)
        let decoded = rest
            .bytes()
            .take(12)
            .position(|b| b == b';')
            .and_then(|j| Some((entity(&rest[1..j])?, j)));
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn entity(name: &str) -> Option<char> {
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some('\u{A0}'),
        _ => {
            let n = name.strip_prefix('#')?;
            let code = match n.strip_prefix(['x', 'X']) {
                Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                None => n.parse().ok()?,
            };
            char::from_u32(code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok() {
        let body = r#"{"success":true,"code":"","message":"","data":{"translateText":"Let&#39;s go &lt;b&gt;","detectLanguage":"zh"}}"#;
        assert_eq!(parse(body).unwrap().as_deref(), Some("Let's go <b>"));
    }

    #[test]
    fn stale_token() {
        assert_eq!(parse("\n\n<!DOCTYPE html><html>").unwrap(), None);
    }

    #[test]
    fn failure() {
        let body = r#"{"success":false,"message":"too long"}"#;
        assert_eq!(parse(body).unwrap_err(), "too long");
    }

    #[test]
    fn entities() {
        assert_eq!(
            unescape_html("Tom &amp; Jerry&#x27;s &quot;show&quot;"),
            "Tom & Jerry's \"show\""
        );
        // 不是实体的 & 原样保留，包括多字节字符旁边的
        assert_eq!(unescape_html("A & B &中文; &#xZZ;"), "A & B &中文; &#xZZ;");
        assert_eq!(unescape_html("末尾&"), "末尾&");
    }

    #[test]
    #[ignore = "需要联网"]
    fn live() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        assert!(super::super::contains_cjk(
            &translate(&agent, "Good morning", "zh-CN").unwrap()
        ));
        assert!(!translate(&agent, "早上好", "en").unwrap().is_empty());
    }
}
