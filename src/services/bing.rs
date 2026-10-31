//! Bing 翻译网页接口（cn.bing.com/ttranslatev3）。不需要 key，国内直连；非官方，可能变动。
//! 先从翻译页（约 600KB）取 IG/IID/key/token，缓存后每次翻译只发一个请求。

use super::{Res, TokenCache, json, read};
use std::time::Duration;
use ureq::Agent;

const PAGE: &str = "https://cn.bing.com/translator";
const API: &str = "https://cn.bing.com/ttranslatev3";
/// 不带浏览器 User-Agent 会返回 401。
const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
/// 页面里写的有效期是 1 小时，留点余量。
const TTL: Duration = Duration::from_secs(50 * 60);

static AUTH: TokenCache<Auth> = TokenCache::new();

#[derive(Clone, Debug, PartialEq)]
struct Auth {
    ig: String,
    iid: String,
    key: String,
    token: String,
}

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<String> {
    let to = lang(target);
    AUTH.call(
        TTL,
        || {
            parse_auth(&read(
                agent.get(PAGE).header("User-Agent", BROWSER_UA).call(),
            )?)
        },
        |a| {
            let resp = agent
                .post(API)
                .query("isVertical", "1")
                .query("IG", &a.ig)
                .query("IID", &a.iid)
                .header("User-Agent", BROWSER_UA)
                .send_form([
                    ("fromLang", "auto-detect"),
                    ("to", to),
                    ("text", text),
                    ("token", a.token.as_str()),
                    ("key", a.key.as_str()),
                ]);
            parse(&read(resp)?)
        },
    )
}

fn lang(target: &str) -> &str {
    if target == "zh-CN" { "zh-Hans" } else { target }
}

/// `start` 之后、下一个 `end` 之前的内容。
fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let rest = &s[s.find(start)? + start.len()..];
    Some(&rest[..rest.find(end)?])
}

/// 页面里有 `IG:"…"`、`data-iid="…"`、`params_AbusePreventionHelper = [key,"token",ttl]`。
/// 每项一次 find，整体 O(页面长度)。
fn parse_auth(page: &str) -> Res<Auth> {
    let helper = between(page, "params_AbusePreventionHelper = [", "]");
    let mut parts = helper.into_iter().flat_map(|l| l.split(','));
    let (Some(ig), Some(iid), Some(key), Some(token)) = (
        between(page, "IG:\"", "\""),
        between(page, "data-iid=\"", "\""),
        parts.next(),
        parts.next(),
    ) else {
        return Err("Bing 翻译页里找不到令牌".into());
    };
    Ok(Auth {
        ig: ig.into(),
        iid: iid.into(),
        key: key.trim().into(),
        token: token.trim().trim_matches('"').into(),
    })
}

/// 成功时返回 `[{"translations":[{"text":"…"}]}]`；令牌失效时返回 `{"statusCode":205}`。
fn parse(body: &str) -> Res<Option<String>> {
    let v = json("Bing", body)?;
    if v["statusCode"].is_number() {
        return Ok(None);
    }
    v[0]["translations"][0]["text"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| Some(s.to_string()))
        .ok_or_else(|| "Bing 返回中没有译文".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_from_page() {
        let page = r#"<div id="rich_tta" data-iid="translator.5023"></div>
            <script>_G={IG:"4C78E88B",x:1};var params_AbusePreventionHelper = [1793458392581,"tok-en_1",3600000];</script>"#;
        assert_eq!(
            parse_auth(page).unwrap(),
            Auth {
                ig: "4C78E88B".into(),
                iid: "translator.5023".into(),
                key: "1793458392581".into(),
                token: "tok-en_1".into(),
            }
        );
        assert!(parse_auth("<html></html>").is_err());
    }

    #[test]
    fn translation() {
        let body = r#"[{"translations":[{"text":"早上好","to":"zh-Hans"}],"detectedLanguage":{"language":"en"}}]"#;
        assert_eq!(parse(body).unwrap().as_deref(), Some("早上好"));
    }

    #[test]
    fn stale_token() {
        assert_eq!(
            parse(r#"{"statusCode":205,"errorMessage":""}"#).unwrap(),
            None
        );
    }

    #[test]
    #[ignore = "需要联网"]
    fn live() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        assert!(super::super::contains_cjk(
            &translate(&agent, "Good morning", "zh-CN").unwrap()
        ));
        // 第二次走缓存的令牌
        assert!(!translate(&agent, "早上好", "en").unwrap().is_empty());
    }
}
