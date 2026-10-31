//! MyMemory：https://mymemory.translated.net/doc/spec.php 。不需要 key，匿名有每日额度。

use super::{Res, get, json};
use ureq::Agent;

const URL: &str = "https://api.mymemory.translated.net/get";

pub fn translate(agent: &Agent, text: &str, target: &str) -> Res<String> {
    let pair = format!("autodetect|{target}");
    let body = get(agent, URL, &[("q", text), ("langpair", &pair)])?;
    parse(&body)
}

fn parse(body: &str) -> Res<String> {
    let v = json("MyMemory", body)?;
    if v["responseStatus"].as_u64() != Some(200) {
        return Err(v["responseDetails"]
            .as_str()
            .unwrap_or("未知错误")
            .to_string());
    }
    v["responseData"]["translatedText"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "MyMemory 返回中没有译文".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok() {
        let body = r#"{"responseData":{"translatedText":"大家好"},"responseStatus":200}"#;
        assert_eq!(parse(body).unwrap(), "大家好");
    }

    #[test]
    fn quota_error() {
        let body = r#"{"responseData":{"translatedText":""},"responseStatus":429,"responseDetails":"QUOTA EXCEEDED"}"#;
        assert!(parse(body).unwrap_err().contains("QUOTA"));
    }
}
