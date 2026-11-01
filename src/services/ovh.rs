//! OVHcloud AI Endpoints：免 key、兼容 OpenAI 的大模型接口，纠错的主力（LanguageTool 只在这里
//! 全部失败时兜底），能查出规则查不出的时态、词形、搭配错误。匿名限流是每个 IP、每个模型每分钟 2 次（429 带 retry-after），
//! 所以几个模型轮流用，被限流或失败的模型先歇着。实测记录见 服务调研.md。

use super::languagetool::Issue;
use super::{Res, json};
use serde_json::json;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use ureq::Agent;

const URL: &str = "https://oai.endpoints.kepler.ai.cloud.ovh.net/v1/chat/completions";

/// (模型, reasoning_effort, 界面上显示的名字)，按纠错质量排序。
/// gpt-oss 和 Qwen3 是推理模型，不压低推理量时 token 全花在思考上，返回空内容。
const MODELS: &[(&str, &str, &str)] = &[
    ("gpt-oss-120b", "low", "gpt-oss-120b"),
    ("gpt-oss-20b", "low", "gpt-oss-20b"),
    ("Mistral-Small-3.2-24B-Instruct-2506", "", "Mistral Small"),
    ("Qwen3.8-27B", "none", "Qwen3.8"),
];

/// 实测单次 1~10 秒；超过这个时间换下一个模型
const TIMEOUT: Duration = Duration::from_secs(15);
/// 失败且没有 retry-after 时歇一分钟，正好是限流窗口
const COOLDOWN: Duration = Duration::from_secs(60);

const PROMPT: &str = "You are an English grammar corrector for a language learner. \
Return ONLY a JSON object: {\"corrected\": \"<full corrected text>\", \"edits\": \
[{\"original\": \"<exact substring of the input>\", \"replacement\": \"<fix>\", \
\"message\": \"<short reason>\"}]}. List edits in the order they appear. \
Make each edit as small as possible: one word or a short phrase, one reason per edit; \
never one edit for a whole sentence. \
Fix grammar, tense, word form and word choice; keep the author's meaning. \
Do not translate, delete or rewrite non-English text. \
If the text is already correct, return an empty edits array.";

/// 每个模型下次可以再用的时间
static READY_AT: Mutex<[Option<Instant>; MODELS.len()]> = Mutex::new([None; MODELS.len()]);

fn ready_at() -> MutexGuard<'static, [Option<Instant>; MODELS.len()]> {
    READY_AT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// 返回找到的问题和所用模型的名字。按顺序试，失败的模型冷却后才会再用。
pub fn check(agent: &Agent, text: &str) -> Res<(Vec<Issue>, &'static str)> {
    let mut errors = Vec::new();
    for (i, &(model, effort, label)) in MODELS.iter().enumerate() {
        if ready_at()[i].is_some_and(|t| Instant::now() < t) {
            continue;
        }
        let r = request(agent, model, effort, text)
            .and_then(|content| parse(text, &content).map_err(|e| (e, COOLDOWN)));
        match r {
            Ok(issues) => return Ok((issues, label)),
            Err((e, wait)) => {
                ready_at()[i] = Some(Instant::now() + wait);
                errors.push(format!("{label}：{e}"));
            }
        }
    }
    if errors.is_empty() {
        return Err("AI 纠错的模型都在限流冷却中，稍后再试".into());
    }
    Err(errors.join("\n"))
}

/// 返回模型回复的内容；失败时同时返回这个模型要冷却多久。
fn request(
    agent: &Agent,
    model: &str,
    effort: &str,
    text: &str,
) -> Result<String, (String, Duration)> {
    let mut body = json!({
        "model": model,
        "temperature": 0,
        "max_tokens": 2000,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": PROMPT},
            {"role": "user", "content": text},
        ],
    });
    if !effort.is_empty() {
        body["reasoning_effort"] = effort.into();
    }
    let fail = |e: String| (e, COOLDOWN);
    // 429 要读 retry-after，所以不把非 2xx 当成错误
    let mut resp = agent
        .post(URL)
        .config()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .header("Content-Type", "application/json")
        .send(body.to_string())
        .map_err(|e| fail(e.to_string()))?;
    let status = resp.status();
    if status == 429 {
        let wait = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok()?.parse().ok())
            .map_or(COOLDOWN, Duration::from_secs);
        return Err(("被限流".into(), wait));
    }
    let raw = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| fail(e.to_string()))?;
    if !status.is_success() {
        return Err(fail(format!("HTTP {status}")));
    }
    let v = json("OVH", &raw).map_err(fail)?;
    v["choices"][0]["message"]["content"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(String::from)
        .ok_or_else(|| fail("返回内容为空".into()))
}

/// 模型给的是原文片段，按顺序在原文里找位置，从上一处的结尾往后找，同一个词出现多次也不会错位。
/// 找到的片段合计只扫一遍原文；找不到（模型改写了原文或顺序乱了）、没有改动的条目丢掉。
fn parse(text: &str, content: &str) -> Res<Vec<Issue>> {
    let v = json("OVH", content)?;
    let edits = v["edits"].as_array().ok_or("OVH 返回中没有 edits")?;
    let mut pos = 0;
    let mut issues = Vec::new();
    for e in edits {
        let (Some(original), Some(replacement)) =
            (e["original"].as_str(), e["replacement"].as_str())
        else {
            continue;
        };
        if original.is_empty() || original == replacement {
            continue;
        }
        let Some(at) = text[pos..].find(original) else {
            continue;
        };
        let offset = pos + at;
        pos = offset + original.len();
        issues.push(Issue {
            offset,
            len: original.len(),
            message: e["message"].as_str().unwrap_or_default().to_string(),
            replacements: vec![replacement.to_string()],
        });
    }
    Ok(issues)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_locates_edits_in_order() {
        let text = "i go to school and i buy a book";
        let content = r#"{"corrected": "...", "edits": [
            {"original": "i go", "replacement": "I went", "message": "tense"},
            {"original": "i buy", "replacement": "I bought", "message": "tense"}
        ]}"#;
        let issues = parse(text, content).unwrap();
        let found: Vec<(&str, &str)> = issues
            .iter()
            .map(|i| (i.original(text), i.replacements[0].as_str()))
            .collect();
        assert_eq!(found, [("i go", "I went"), ("i buy", "I bought")]);
        // 第二个 "i" 要从第一处之后找
        assert_eq!(issues[1].offset, 19);
    }

    #[test]
    fn parse_drops_unusable_edits() {
        let text = "我想说 this are fine";
        let content = r#"{"edits": [
            {"original": "this are", "replacement": "this is", "message": "agreement"},
            {"original": "fine", "replacement": "fine", "message": "no-op"},
            {"original": "rewritten text", "replacement": "x", "message": "not in input"},
            {"original": "", "replacement": "x", "message": "empty"},
            {"replacement": "x"}
        ]}"#;
        let issues = parse(text, content).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].original(text), "this are");
    }

    #[test]
    fn parse_rejects_non_json() {
        assert!(parse("text", "Sure! Here is the fix").is_err());
        assert!(parse("text", r#"{"corrected": "text"}"#).is_err());
    }

    #[test]
    #[ignore = "需要联网"]
    fn live() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        let text =
            "but i modify the pptx before generate last time, so i wish my modify do not be cover";
        let (issues, model) = check(&agent, text).unwrap();
        println!("{model}: {issues:?}");
        assert!(!issues.is_empty());
    }
}
