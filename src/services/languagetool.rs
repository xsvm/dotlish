//! LanguageTool 公共 API：https://dev.languagetool.org/public-http-api
//! 限制：每 IP 每分钟 20 次、75KB；单次 20KB；禁止自动化请求。

use super::{Res, json, post_form};
use ureq::Agent;

const URL: &str = "https://api.languagetool.org/v2/check";

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    /// 字节偏移
    pub offset: usize,
    /// 字节长度
    pub len: usize,
    pub message: String,
    pub replacements: Vec<String>,
}

impl Issue {
    pub fn original<'a>(&self, text: &'a str) -> &'a str {
        &text[self.offset..self.offset + self.len]
    }
}

pub fn check(agent: &Agent, text: &str) -> Res<Vec<Issue>> {
    let body = post_form(agent, URL, &[("text", text), ("language", "auto")])?;
    parse(text, &body)
}

/// 把升序的 UTF-16 偏移批量换成字节偏移。只遍历文本一次：O(n + m)。
fn utf16_to_byte_offsets(s: &str, units: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(units.len());
    let mut chars = s.chars();
    let (mut unit, mut byte) = (0, 0);
    for &target in units {
        while unit < target {
            let Some(c) = chars.next() else { break };
            unit += c.len_utf16();
            byte += c.len_utf8();
        }
        out.push(byte);
    }
    out
}

/// LanguageTool 返回的 offset/length 以 UTF-16 计（Java 字符串）。
/// 排序 O(m log m) + 一次文本遍历 O(n)。
fn parse(text: &str, body: &str) -> Res<Vec<Issue>> {
    let v = json("LanguageTool", body)?;
    let matches = v["matches"]
        .as_array()
        .ok_or("LanguageTool 返回中没有 matches")?;

    // (起点, 终点, 原始 match)，按 UTF-16 单位
    let mut spans: Vec<(usize, usize, &serde_json::Value)> = matches
        .iter()
        .filter_map(|m| {
            let off = m["offset"].as_u64()? as usize;
            Some((off, off + m["length"].as_u64()? as usize, m))
        })
        .collect();
    spans.sort_by_key(|s| s.0);
    // 区间重叠时只保留前一个，否则替换会错位
    spans.dedup_by(|b, a| b.0 < a.1);

    // 去重后区间互不重叠，起点终点交替出现且整体升序，可以一次换算
    let bounds: Vec<usize> = spans.iter().flat_map(|s| [s.0, s.1]).collect();
    let bytes = utf16_to_byte_offsets(text, &bounds);
    let (pairs, _) = bytes.as_chunks::<2>();

    Ok(spans
        .iter()
        .zip(pairs)
        .map(|((_, _, m), &[start, end])| Issue {
            offset: start,
            len: end - start,
            message: m["message"].as_str().unwrap_or_default().to_string(),
            replacements: m["replacements"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|r| r["value"].as_str().map(String::from))
                        .take(3)
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect())
}

/// picks[i] 是 issues[i] 选中的候选下标，None 表示保留原文。issues 需按 offset 升序。O(n)。
pub fn apply(text: &str, issues: &[Issue], picks: &[Option<usize>]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for (issue, pick) in issues.iter().zip(picks) {
        let Some(rep) = pick.and_then(|j| issue.replacements.get(j)) else {
            continue;
        };
        out.push_str(&text[pos..issue.offset]);
        out.push_str(rep);
        pos = issue.offset + issue.len;
    }
    out.push_str(&text[pos..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT: &str = "Their is a 测试 dont work 😀 ok.";
    // offset 以 UTF-16 计：“测试”各占 1 个单位，😀 占 2 个
    const BODY: &str = r#"{"matches":[
        {"offset":14,"length":4,"message":"Missing apostrophe","replacements":[{"value":"doesn't"},{"value":"don't"}]},
        {"offset":27,"length":2,"message":"Typo","replacements":[{"value":"OK"}]},
        {"offset":0,"length":8,"message":"Agreement","replacements":[{"value":"There is"}]}
    ]}"#;

    #[test]
    fn parse_converts_utf16_offsets_and_sorts() {
        let issues = parse(TEXT, BODY).unwrap();
        let originals: Vec<&str> = issues.iter().map(|i| i.original(TEXT)).collect();
        assert_eq!(originals, ["Their is", "dont", "ok"]);
    }

    #[test]
    fn apply_respects_picks() {
        let issues = parse(TEXT, BODY).unwrap();
        assert_eq!(
            apply(TEXT, &issues, &[Some(0), Some(0), Some(0)]),
            "There is a 测试 doesn't work 😀 OK."
        );
        assert_eq!(
            apply(TEXT, &issues, &[None, Some(1), None]),
            "Their is a 测试 don't work 😀 ok."
        );
        assert_eq!(apply(TEXT, &issues, &[None, None, None]), TEXT);
    }

    #[test]
    fn overlapping_matches_keep_first() {
        let body = r#"{"matches":[
            {"offset":0,"length":8,"message":"a","replacements":[{"value":"X"}]},
            {"offset":6,"length":2,"message":"b","replacements":[{"value":"Y"}]}
        ]}"#;
        assert_eq!(parse(TEXT, body).unwrap().len(), 1);
    }

    #[test]
    #[ignore = "需要联网"]
    fn live() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        let issues = check(&agent, "This are a test.").unwrap();
        assert!(!issues.is_empty());
    }
}
