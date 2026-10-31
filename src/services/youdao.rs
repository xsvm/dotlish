//! 有道词典非官方接口（suggest）。不需要 key，国内直连；可能变动。

use super::{Res, get, json};
use ureq::Agent;

const URL: &str = "https://dict.youdao.com/suggest";

pub fn lookup(agent: &Agent, word: &str) -> Res<String> {
    let body = get(
        agent,
        URL,
        &[("num", "1"), ("doctype", "json"), ("q", word)],
    )?;
    parse(word, &body)
}

fn parse(word: &str, body: &str) -> Res<String> {
    let v = json("有道", body)?;
    let entry = &v["data"]["entries"][0];
    // suggest 是前缀联想，第一条不一定是这个词本身
    if !entry["entry"]
        .as_str()
        .is_some_and(|e| e.eq_ignore_ascii_case(word))
    {
        return Err(format!("词典里没有找到“{word}”"));
    }
    entry["explain"]
        .as_str()
        .map(String::from)
        .ok_or_else(|| "有道返回中没有释义".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_entry() {
        let body = r#"{"data":{"entries":[{"explain":"n. 意外发现美好事物的运气","entry":"serendipity"}]}}"#;
        assert_eq!(
            parse("Serendipity", body).unwrap(),
            "n. 意外发现美好事物的运气"
        );
    }

    #[test]
    fn prefix_match_is_not_a_hit() {
        let body = r#"{"data":{"entries":[{"explain":"x","entry":"serendipitous"}]}}"#;
        assert!(parse("serendip", body).is_err());
    }
}
