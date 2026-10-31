//! 有道词典非官方接口（jsonapi）。不需要 key，国内直连；可能变动。
//! 只请求英汉词典（ec）：完整响应约 500KB，限定后约 2KB。

use super::{Res, get, json};
use serde_json::Value;
use ureq::Agent;

const URL: &str = "https://dict.youdao.com/jsonapi";
const ONLY_EC: &str = r#"{"count":99,"dicts":[["ec"]]}"#;

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub word: String,
    /// 例如 `英 /rʌn/ · 美 /rʌn/`，没有音标时为空
    pub phonetic: String,
    /// 每个词性一条，例如 `v. 跑，奔跑；…`
    pub senses: Vec<String>,
    /// 词形变化，例如 `过去式 ran · 现在分词 running`，没有时为空
    pub forms: String,
}

impl Entry {
    /// 复制时的纯文本。
    pub fn to_text(&self) -> String {
        let mut lines = vec![self.word.clone()];
        lines.extend(self.senses.iter().cloned());
        lines.join("\n")
    }
}

pub fn lookup(agent: &Agent, word: &str) -> Res<Entry> {
    let body = get(agent, URL, &[("q", word), ("dicts", ONLY_EC)])?;
    parse(word, &body)
}

/// `l.i` 有时是字符串，有时是字符串数组。
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

fn parse(word: &str, body: &str) -> Res<Entry> {
    let v = json("有道", body)?;
    let w = &v["ec"]["word"][0];
    let senses: Vec<String> = w["trs"]
        .as_array()
        .map(|trs| {
            trs.iter()
                .map(|t| text_of(&t["tr"][0]["l"]["i"]))
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if senses.is_empty() {
        return Err(format!("词典里没有找到“{word}”"));
    }

    let phonetic = [("英", "ukphone"), ("美", "usphone")]
        .iter()
        .filter_map(|(label, key)| {
            w[*key]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(|p| format!("{label} /{p}/"))
        })
        .collect::<Vec<_>>()
        .join(" · ");
    let forms = w["wfs"]
        .as_array()
        .map(|wfs| {
            wfs.iter()
                .filter_map(|f| {
                    let f = &f["wf"];
                    Some(format!("{} {}", f["name"].as_str()?, f["value"].as_str()?))
                })
                .collect::<Vec<_>>()
                .join(" · ")
        })
        .unwrap_or_default();
    let shown = text_of(&w["return-phrase"]["l"]["i"]);

    Ok(Entry {
        word: if shown.is_empty() {
            word.to_string()
        } else {
            shown
        },
        phonetic,
        senses,
        forms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_entry() {
        let body = r#"{"ec":{"word":[{"usphone":"rʌn","ukphone":"rʌn",
            "trs":[{"tr":[{"l":{"i":["v. 跑，奔跑"]}}]},{"tr":[{"l":{"i":["n. 跑步"]}}]}],
            "wfs":[{"wf":{"name":"过去式","value":"ran"}},{"wf":{"name":"现在分词","value":"running"}}],
            "return-phrase":{"l":{"i":"run"}}}]}}"#;
        assert_eq!(
            parse("Run", body).unwrap(),
            Entry {
                word: "run".into(),
                phonetic: "英 /rʌn/ · 美 /rʌn/".into(),
                senses: vec!["v. 跑，奔跑".into(), "n. 跑步".into()],
                forms: "过去式 ran · 现在分词 running".into(),
            }
        );
    }

    #[test]
    fn minimal_entry() {
        let body = r#"{"ec":{"word":[{"trs":[{"tr":[{"l":{"i":["n. 回形针助手"]}}]}]}]}}"#;
        let e = parse("clippy", body).unwrap();
        assert_eq!(
            (e.word.as_str(), e.phonetic.as_str(), e.forms.as_str()),
            ("clippy", "", "")
        );
        assert_eq!(e.to_text(), "clippy\nn. 回形针助手");
    }

    #[test]
    fn not_found() {
        let body = r#"{"input":"asdfqwer","meta":{"dicts":["meta"]}}"#;
        assert!(parse("asdfqwer", body).is_err());
    }

    #[test]
    #[ignore = "需要联网"]
    fn live() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        let e = lookup(&agent, "serendipity").unwrap();
        assert!(!e.senses.is_empty() && !e.phonetic.is_empty());
    }
}
