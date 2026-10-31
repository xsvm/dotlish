//! 单词发音。后台一个线程依次处理：下载有道的 MP3（缓存）、播放；
//! 音频设备只打开一次，新的发音会打断正在播放的。

use crate::services::youdao::{self, Accent};
use rodio::{Decoder, DeviceSinkBuilder, MixerDeviceSink, Player};
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, OnceLock};
use ureq::Agent;

struct Request {
    agent: Agent,
    word: String,
    accent: Accent,
}

/// 每个约 15KB，超过就清空重来
const CACHE_LIMIT: usize = 100;

static QUEUE: OnceLock<Sender<Request>> = OnceLock::new();

/// 读出单词；失败时静默（网络或音频设备问题不影响查词）。
pub fn pronounce(agent: Agent, word: String, accent: Accent) {
    let tx = QUEUE.get_or_init(|| {
        let (tx, rx) = channel();
        std::thread::spawn(move || worker(rx));
        tx
    });
    let _ = tx.send(Request {
        agent,
        word,
        accent,
    });
}

fn worker(rx: Receiver<Request>) {
    let mut cache: HashMap<(String, Accent), Arc<[u8]>> = HashMap::new();
    let mut device: Option<MixerDeviceSink> = None;
    // 留着正在播放的，被替换（drop）时自动停止
    let mut _playing: Option<Player> = None;

    for Request {
        agent,
        word,
        accent,
    } in rx
    {
        let key = (word.to_ascii_lowercase(), accent);
        let mp3 = match cache.get(&key) {
            Some(b) => b.clone(),
            None => {
                let Ok(bytes) = youdao::voice(&agent, &word, accent) else {
                    continue;
                };
                if cache.len() >= CACHE_LIMIT {
                    cache.clear();
                }
                cache.entry(key).or_insert(bytes.into()).clone()
            }
        };
        if device.is_none() {
            device = DeviceSinkBuilder::open_default_sink().ok().map(|mut d| {
                d.log_on_drop(false);
                d
            });
        }
        let Some(d) = &device else { continue };
        let Ok(source) = Decoder::try_from(Cursor::new(mp3)) else {
            continue;
        };
        let player = Player::connect_new(d.mixer());
        player.append(source);
        _playing = Some(player);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rodio::Source;

    #[test]
    #[ignore = "需要联网"]
    fn decodes_youdao_mp3() {
        let agent = crate::services::agent(&Default::default()).unwrap();
        let mp3 = youdao::voice(&agent, "hello", Accent::Us).unwrap();
        let source = Decoder::try_from(Cursor::new(mp3)).unwrap();
        assert!(source.sample_rate().get() > 0);
        assert!(source.count() > 1000);
    }
}
