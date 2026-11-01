//! OVHcloud AI Endpoints：免 key、兼容 OpenAI 的大模型接口，纠错的主力（LanguageTool 只在这里
//! 全部失败时兜底），能查出规则查不出的时态、词形、搭配错误。匿名限流是每个 IP、每个模型每分钟 2 次（429 带 retry-after），
//! 所以几个模型轮流用，被限流或失败的模型先歇着。同一模型的耗时忽快忽慢（实测 4~30 秒），
//! 所以慢了就加发下一个模型，谁先答用谁。实测记录见 服务调研.md。

use super::languagetool::Issue;
use super::{Res, json};
use crate::config::language_name;
use serde_json::json;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
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

/// 单个模型最多等这么久
const TIMEOUT: Duration = Duration::from_secs(15);
/// 已发出的模型这么久还没回复，就加发下一个。只在慢的时候多用一次限流额度。
const HEDGE: Duration = Duration::from_secs(3);
/// 失败且没有 retry-after 时歇一分钟，正好是限流窗口
const COOLDOWN: Duration = Duration::from_secs(60);

/// 模型自己识别文字的主要语言并只改这种语言；说明、语言名和译文用 `native` 写。
/// 要求单行 JSON：有的模型默认缩进输出，多出三成 token。
fn prompt(native: &str) -> String {
    format!(
        "You are a writing corrector for a language learner whose native language is {native}. \
Identify the main language of the text (ignore short fragments in other languages) and correct only that language. \
Return ONLY a compact single-line JSON object (no indentation): \
{{\"language\": \"<name of the main language, written in {native}>\", \
\"edits\": [{{\"original\": \"<exact substring of the input>\", \"replacement\": \"<fix>\", \
\"message\": \"<short reason, written in {native}>\"}}], \
\"translation\": \"<the corrected text translated into {native}, or an empty string if the main language is {native}>\"}}. \
List edits in the order they appear. \
Make each edit as small as possible: one word or a short phrase, one reason per edit; \
never one edit for a whole sentence. \
Fix spelling, grammar, tense, word form and word choice; keep the author's meaning. \
Do not translate, delete or rewrite text in other languages. \
If the text is already correct, return an empty edits array."
    )
}

/// 一次纠错的结果
pub struct Checked {
    pub issues: Vec<Issue>,
    /// 模型识别出的语言名（用母语写）；LanguageTool 兜底时为空
    pub language: String,
    /// 改正后整句的译文；文字本来就是母语或没有返回时为空
    pub translation: String,
    /// 界面上显示的服务名
    pub service: &'static str,
}

/// 每个模型下次可以再用的时间
static READY_AT: Mutex<[Option<Instant>; MODELS.len()]> = Mutex::new([None; MODELS.len()]);

fn ready_at() -> MutexGuard<'static, [Option<Instant>; MODELS.len()]> {
    READY_AT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// `native` 是说明和译文的语言代码。按质量顺序发请求：上一个失败立刻换下一个，
/// 超过 HEDGE 没回复就再加发一个，返回最先成功的结果。没用上的请求在后台跑完，失败照样冷却。
pub fn check(agent: &Agent, text: &str, native: &str) -> Res<Checked> {
    let prompt: Arc<str> = prompt(language_name(native)).into();
    let text: Arc<str> = text.into();
    let (tx, rx) = mpsc::channel();
    let (mut next, mut running) = (0, 0);
    let mut errors = Vec::new();
    loop {
        let now = Instant::now();
        let ready = (next..MODELS.len()).find(|&i| ready_at()[i].is_none_or(|t| now >= t));
        next = ready.map_or(MODELS.len(), |i| i + 1);
        if let Some(i) = ready {
            running += 1;
            let (tx, agent, prompt, text) =
                (tx.clone(), agent.clone(), prompt.clone(), text.clone());
            std::thread::spawn(move || {
                let _ = tx.send((i, attempt(&agent, i, &prompt, &text)));
            });
        }
        if running == 0 {
            break;
        }
        // 还有备用模型时只等 HEDGE，否则等到有结果为止
        let got = if next < MODELS.len() {
            rx.recv_timeout(HEDGE).ok()
        } else {
            rx.recv().ok()
        };
        match got {
            None => {}
            Some((_, Ok(r))) => return Ok(r),
            Some((i, Err(e))) => {
                running -= 1;
                errors.push(format!("{}：{e}", MODELS[i].2));
            }
        }
    }
    if errors.is_empty() {
        return Err("AI 纠错的模型都在限流冷却中，稍后再试".into());
    }
    Err(errors.join("\n"))
}

/// 用第 `i` 个模型纠错一次；失败时让它冷却。
fn attempt(agent: &Agent, i: usize, prompt: &str, text: &str) -> Res<Checked> {
    let (model, effort, label) = MODELS[i];
    let r = request(agent, model, effort, prompt, text)
        .and_then(|content| parse(text, &content).map_err(|e| (e, COOLDOWN)));
    match r {
        Ok((issues, language, translation)) => Ok(Checked {
            issues,
            language,
            translation,
            service: label,
        }),
        Err((e, wait)) => {
            ready_at()[i] = Some(Instant::now() + wait);
            Err(e)
        }
    }
}

/// 返回模型回复的内容；失败时同时返回这个模型要冷却多久。
fn request(
    agent: &Agent,
    model: &str,
    effort: &str,
    prompt: &str,
    text: &str,
) -> Result<String, (String, Duration)> {
    let mut body = json!({
        "model": model,
        "temperature": 0,
        "max_tokens": 2000,
        "response_format": {"type": "json_object"},
        "messages": [
            {"role": "system", "content": prompt},
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
/// 通常合计只扫一遍原文；找不到（模型改写了原文或顺序乱了）、没有改动的条目丢掉。
/// 返回问题列表、识别出的语言名和整句译文。
fn parse(text: &str, content: &str) -> Res<(Vec<Issue>, String, String)> {
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
        let Some(offset) = find_from(text, pos, original) else {
            continue;
        };
        pos = offset + original.len();
        issues.push(Issue {
            offset,
            len: original.len(),
            message: e["message"].as_str().unwrap_or_default().to_string(),
            replacements: vec![replacement.to_string()],
        });
    }
    let field = |k: &str| v[k].as_str().unwrap_or_default().trim().to_string();
    Ok((issues, field("language"), field("translation")))
}

/// 从 `from` 往后找 `needle`，优先整词匹配（"i" 不会落在 "time" 里），没有整词时退回普通子串。
fn find_from(text: &str, from: usize, needle: &str) -> Option<usize> {
    let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let (head, tail) = (
        word(needle.chars().next()),
        word(needle.chars().next_back()),
    );
    let hay = &text[from..];
    hay.match_indices(needle)
        .map(|(i, _)| from + i)
        .find(|&at| {
            !(head && word(text[..at].chars().next_back()))
                && !(tail && word(text[at + needle.len()..].chars().next()))
        })
        .or_else(|| hay.find(needle).map(|i| from + i))
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
        let (issues, language, translation) = parse(text, content).unwrap();
        assert_eq!((language.as_str(), translation.as_str()), ("", ""));
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
        let (issues, ..) = parse(text, content).unwrap();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].original(text), "this are");
    }

    #[test]
    fn parse_prefers_whole_words() {
        let text = "before generate last time, so i wish";
        let content = r#"{"edits": [
            {"original": "generate", "replacement": "generating", "message": ""},
            {"original": "i", "replacement": "I", "message": ""}
        ]}"#;
        let (issues, ..) = parse(text, content).unwrap();
        assert_eq!(issues[1].offset, text.find(" i ").unwrap() + 1);
        // 没有整词时退回子串，中文这样不分词的文本也能找到
        assert_eq!(find_from("我想说这个", 0, "说"), Some(6));
    }

    #[test]
    fn parse_reads_language_and_translation() {
        let content = r#"{"language": "英语", "edits": [], "translation": " 我去了学校。 "}"#;
        let (issues, language, translation) = parse("I went to school.", content).unwrap();
        assert!(issues.is_empty());
        assert_eq!(language, "英语");
        assert_eq!(translation, "我去了学校。");
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
        let t = Instant::now();
        let r = check(&agent, text, "zh-CN").unwrap();
        println!(
            "{:?} {} {}: {:?}\n{}",
            t.elapsed(),
            r.service,
            r.language,
            r.issues,
            r.translation
        );
        assert!(!r.issues.is_empty());
        assert!(!r.translation.is_empty());
    }
}
