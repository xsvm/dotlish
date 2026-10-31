//! Google 翻译非官方接口（client=gtx）。不需要 key，但随时可能失效；国内需要代理。

use super::{Res, get, json};
use ureq::Agent;

const URL: &str = "https://translate.googleapis.com/translate_a/single";

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<String> {
    let body = get(
        agent,
        URL,
        &[
            ("client", "gtx"),
            ("sl", "auto"),
            ("tl", target),
            ("dt", "t"),
            ("q", text),
        ],
    )?;
    parse(&body)
}

/// 返回形如 [[["译文1","原文1",...],["译文2","原文2",...]], null, "en", ...]
fn parse(body: &str) -> Res<String> {
    let v = json("Google", body)?;
    let parts = v[0].as_array().ok_or("Google 返回中没有译文")?;
    let out: String = parts.iter().filter_map(|p| p[0].as_str()).collect();
    if out.is_empty() {
        Err("Google 返回空译文".into())
    } else {
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_segments() {
        let body =
            r#"[[["你好。","Hello.",null,null,10],["世界","World",null,null,10]],null,"en"]"#;
        assert_eq!(parse(body).unwrap(), "你好。世界");
    }

    #[test]
    fn rejects_html() {
        assert!(parse("<html>sorry</html>").is_err());
    }
}
