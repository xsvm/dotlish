//! 火山翻译浏览器插件接口。不需要 key，国内直连；非官方，可能变动。

use super::{Res, json, post_json};
use serde_json::json;
use ureq::Agent;

const URL: &str = "https://translate.volcengine.com/crx/translate/v1/";

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<String> {
    let body = json!({
        "source_language": "detect",
        "target_language": lang(target),
        "text": text,
    });
    parse(&post_json(agent, URL, &body)?)
}

/// 火山不认识的语言码不报错，而是原样返回原文，所以必须换算对。
fn lang(target: &str) -> &str {
    if target == "zh-CN" { "zh" } else { target }
}

fn parse(body: &str) -> Res<String> {
    let v = json("火山", body)?;
    if v["base_resp"]["status_code"].as_i64() != Some(0) {
        return Err(v["base_resp"]["status_message"]
            .as_str()
            .unwrap_or("未知错误")
            .to_string());
    }
    v["translation"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| "火山返回中没有译文".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok() {
        let body = r#"{"translation":"你好","detected_language":"en","base_resp":{"status_code":0,"status_message":""}}"#;
        assert_eq!(parse(body).unwrap(), "你好");
    }

    #[test]
    fn error_status() {
        let body = r#"{"base_resp":{"status_code":-1,"status_message":"text too long"}}"#;
        assert_eq!(parse(body).unwrap_err(), "text too long");
    }

    #[test]
    fn lang_codes() {
        assert_eq!(lang("zh-CN"), "zh");
        assert_eq!(lang("ja"), "ja");
    }

    #[test]
    #[ignore = "需要联网"]
    fn live() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        assert!(super::super::contains_cjk(
            &translate(&agent, "Good morning", "zh-CN").unwrap()
        ));
    }
}
