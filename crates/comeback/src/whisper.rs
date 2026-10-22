//! Local transcription through `whisper-cli` (whisper.cpp) and the ggml models already on disk.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use comeback_core::model::Word;

use crate::gst::wait_timeout;

/// Longest a transcription may run before we give up and ship the bundle without one.
const TRANSCRIBE_LIMIT: Duration = Duration::from_secs(600);

/// The ggml file for a model name: `COMEBACK_WHISPER_MODEL` (a path) wins, then comeback's own
/// model directory, then the shared `~/.local/share/whisper-models`.
pub fn model_path(name: &str) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("COMEBACK_WHISPER_MODEL") {
        return Some(PathBuf::from(p));
    }
    let data = crate::data_dir();
    [data.join("comeback/models"), data.join("whisper-models")]
        .into_iter()
        .map(|d| d.join(format!("ggml-{name}.bin")))
        .find(|p| p.is_file())
}

/// Runs `whisper-cli` on a 16 kHz mono WAV. `scratch` receives its JSON output.
pub fn transcribe(wav: &Path, scratch: &Path, model: &Path) -> Result<Vec<Word>> {
    let cli = std::env::var("COMEBACK_WHISPER_CLI").unwrap_or_else(|_| "whisper-cli".into());
    let prefix = scratch.join("transcript");
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get().min(4));
    let mut cmd = Command::new(&cli);
    // A transcription can run for minutes: do not leave it burning CPU if comeback is killed.
    crate::gst::die_with_parent(&mut cmd, libc::SIGKILL);
    let mut child = cmd
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(wav)
        .args([
            "-ojf",
            "-ml",
            "1",
            "-sow",
            "-np",
            "-t",
            &threads.to_string(),
            "-of",
        ])
        .arg(&prefix)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {cli}"))?;
    let Some(status) = wait_timeout(&mut child, TRANSCRIBE_LIMIT) else {
        let _ = child.kill();
        let _ = child.wait();
        bail!("{cli} ran longer than {} s", TRANSCRIBE_LIMIT.as_secs());
    };
    if !status.success() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            std::io::Read::read_to_string(&mut e, &mut err).ok();
        }
        let tail = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("");
        bail!("{cli} exited with {status}: {}", tail.trim());
    }
    let json =
        std::fs::read_to_string(prefix.with_extension("json")).context("reading whisper output")?;
    parse_words(&json)
}

/// Words from `whisper-cli -ojf -ml 1 -sow` output, with millisecond offsets.
pub fn parse_words(json: &str) -> Result<Vec<Word>> {
    let doc: serde_json::Value =
        serde_json::from_str(json).context("whisper output is not JSON")?;
    let segments = doc
        .get("transcription")
        .and_then(|t| t.as_array())
        .context("whisper output has no \"transcription\" list")?;
    let mut words: Vec<Word> = Vec::new();
    // Inside a multi-word tag such as "(electronic music)", which -ml 1 splits per word.
    let mut tag_closer: Option<char> = None;
    for seg in segments {
        let text = seg.get("text").and_then(|t| t.as_str()).unwrap_or("");
        let from = seg["offsets"]["from"].as_u64().unwrap_or(0);
        let to = seg["offsets"]["to"].as_u64().unwrap_or(from);
        let bare = text.trim();
        if let Some(closer) = tag_closer {
            if bare.ends_with(closer) {
                tag_closer = None;
            }
            continue;
        }
        if bare.is_empty() || is_marker(bare) {
            continue;
        }
        if let Some(closer) = tag_opener(bare) {
            tag_closer = Some(closer);
            continue;
        }
        let continues = !text.starts_with(' ') && !words.is_empty();
        let punctuation = bare.chars().all(|c| c.is_ascii_punctuation());
        if continues || (punctuation && !words.is_empty()) {
            let last = words.last_mut().expect("checked non-empty");
            last.w.push_str(bare);
            last.end_ms = last.end_ms.max(to);
        } else {
            words.push(Word {
                w: bare.to_string(),
                start_ms: from,
                end_ms: to.max(from),
            });
        }
    }
    Ok(words)
}

/// The closing character when `text` opens a `(...)` or `[...]` tag that it does not close.
fn tag_opener(text: &str) -> Option<char> {
    match text.chars().next()? {
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    }
}

/// `[BLANK_AUDIO]`, `[_BEG_]`, `(music)` and similar non-speech tags.
fn is_marker(text: &str) -> bool {
    (text.starts_with('[') && text.ends_with(']')) || (text.starts_with('(') && text.ends_with(')'))
}

/// True when at least 95% of 20 ms windows have an RMS below the speech floor.
pub fn is_silent(samples: &[i16]) -> bool {
    const WINDOW: usize = 320;
    const SPEECH_FLOOR_RMS: f64 = 150.0;
    let windows: Vec<&[i16]> = samples.chunks(WINDOW).collect();
    if windows.is_empty() {
        return true;
    }
    let quiet = windows
        .iter()
        .filter(|w| {
            let sum: f64 = w.iter().map(|&s| f64::from(s).powi(2)).sum();
            (sum / w.len() as f64).sqrt() < SPEECH_FLOOR_RMS
        })
        .count();
    quiet as f64 / windows.len() as f64 >= 0.95
}

/// The samples of a mono or multi-channel WAV as 16-bit values, whatever sample format the file
/// holds. Reading float or 24-bit data as if it were 16-bit turns quiet noise into loud garbage.
pub fn read_samples(wav: &[u8]) -> Result<Vec<i16>> {
    if wav.len() < 12 || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        bail!("not a WAV file");
    }
    // (format tag, bits per sample) from the `fmt ` chunk; tag 0xFFFE carries the real tag at the end.
    let mut format: Option<(u16, u16)> = None;
    let mut at = 12;
    while at + 8 <= wav.len() {
        let size = u32::from_le_bytes(wav[at + 4..at + 8].try_into().expect("4 bytes")) as usize;
        let body = at + 8;
        let id = &wav[at..at + 4];
        if id == b"fmt " && body + 16 <= wav.len() {
            let u16_at = |o: usize| u16::from_le_bytes([wav[body + o], wav[body + o + 1]]);
            let mut tag = u16_at(0);
            if tag == 0xFFFE && size >= 26 && body + 26 <= wav.len() {
                tag = u16_at(24);
            }
            format = Some((tag, u16_at(14)));
        } else if id == b"data" {
            let Some((tag, bits)) = format else {
                bail!("WAV has no fmt chunk before its data");
            };
            // A recorder that was killed leaves the size field at its placeholder.
            let end = body.saturating_add(size).min(wav.len());
            return decode(&wav[body..end], tag, bits);
        }
        at = body.saturating_add(size).saturating_add(size & 1);
    }
    bail!("WAV has no data chunk")
}

fn decode(data: &[u8], tag: u16, bits: u16) -> Result<Vec<i16>> {
    const PCM: u16 = 1;
    const FLOAT: u16 = 3;
    Ok(match (tag, bits) {
        (PCM, 16) => data
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect(),
        (PCM, 24) => data
            .as_chunks::<3>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes([b[1], b[2]]))
            .collect(),
        (PCM, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| (i32::from_le_bytes(*b) >> 16) as i16)
            .collect(),
        (FLOAT, 32) => data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| (f32::from_le_bytes(*b).clamp(-1.0, 1.0) * 32767.0) as i16)
            .collect(),
        _ => bail!("unsupported WAV sample format (tag {tag}, {bits} bits)"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(from: u64, to: u64, text: &str) -> String {
        format!(
            r#"{{"offsets":{{"from":{from},"to":{to}}},"text":{},"tokens":[]}}"#,
            serde_json::to_string(text).unwrap()
        )
    }
    fn doc(segs: &[String]) -> String {
        format!(r#"{{"transcription":[{}]}}"#, segs.join(","))
    }

    #[test]
    fn words_come_from_segments_with_offsets() {
        let json = doc(&[
            seg(0, 320, ""),
            seg(320, 370, " And"),
            seg(370, 690, " so"),
            seg(690, 700, " my"),
        ]);
        let words = parse_words(&json).unwrap();
        let got: Vec<_> = words
            .iter()
            .map(|w| (w.w.as_str(), w.start_ms, w.end_ms))
            .collect();
        assert_eq!(got, [("And", 320, 370), ("so", 370, 690), ("my", 690, 700)]);
    }

    #[test]
    fn markers_and_blank_audio_are_dropped() {
        let json = doc(&[
            seg(0, 10, " [BLANK_AUDIO]"),
            seg(10, 20, " (music)"),
            seg(20, 30, "[_BEG_]"),
            seg(30, 90, " Hello"),
        ]);
        let words = parse_words(&json).unwrap();
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].w, "Hello");
    }

    #[test]
    fn tags_split_across_words_are_dropped_whole() {
        let json = doc(&[
            seg(0, 100, " (electronic"),
            seg(100, 200, " music)"),
            seg(200, 300, " [soft"),
            seg(300, 400, " piano]"),
            seg(400, 500, " Hello"),
        ]);
        let words = parse_words(&json).unwrap();
        assert_eq!(words.len(), 1, "{words:?}");
        assert_eq!(words[0].w, "Hello");
    }

    #[test]
    fn punctuation_and_word_pieces_attach_to_the_previous_word() {
        let json = doc(&[
            seg(0, 100, " Wait"),
            seg(100, 120, "."),
            seg(150, 300, " Reg"),
            seg(300, 400, "ression"),
        ]);
        let words = parse_words(&json).unwrap();
        let got: Vec<_> = words
            .iter()
            .map(|w| (w.w.as_str(), w.start_ms, w.end_ms))
            .collect();
        assert_eq!(got, [("Wait.", 0, 120), ("Regression", 150, 400)]);
    }

    #[test]
    fn rejects_output_that_is_not_whisper_json() {
        assert!(parse_words("not json").is_err());
        assert!(parse_words(r#"{"nope":1}"#).is_err());
    }

    fn tone(n: usize, amp: f64) -> Vec<i16> {
        (0..n)
            .map(|i| (amp * (i as f64 * 0.3).sin()) as i16)
            .collect()
    }

    #[test]
    fn silence_is_detected() {
        assert!(is_silent(&[0i16; 16_000]));
        assert!(is_silent(&tone(16_000, 20.0)), "noise floor is silent");
        assert!(!is_silent(&tone(16_000, 8_000.0)));
        assert!(is_silent(&[]), "no samples is silent");
    }

    #[test]
    fn a_short_burst_in_long_silence_still_counts_as_speech() {
        let mut s = vec![0i16; 16_000 * 4];
        s.splice(16_000..16_000 + 16_000, tone(16_000, 8_000.0));
        assert!(!is_silent(&s));
    }

    fn wav(tag: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let block = bits / 8;
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend(((36 + data.len()) as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(tag.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(16_000u32.to_le_bytes());
        wav.extend((16_000 * u32::from(block)).to_le_bytes());
        wav.extend(block.to_le_bytes());
        wav.extend(bits.to_le_bytes());
        wav.extend(b"data");
        wav.extend((data.len() as u32).to_le_bytes());
        wav.extend(data);
        wav
    }

    #[test]
    fn reads_pcm16_from_a_wav() {
        let samples = [1i16, -2, 300, -400];
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        assert_eq!(read_samples(&wav(1, 16, &data)).unwrap(), samples);
        assert!(read_samples(b"junk").is_err());
    }

    /// The bug: float WAV read as 16-bit makes a 0.002 noise floor look like loud speech.
    #[test]
    fn float_wav_is_scaled_not_misread_as_pcm16() {
        let quiet: Vec<u8> = [0.002f32, -0.002, 0.0015, -0.001]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let got = read_samples(&wav(3, 32, &quiet)).unwrap();
        assert_eq!(got.len(), 4, "one sample per float, not two per float");
        assert!(got.iter().all(|s| s.abs() < 100), "{got:?}");
        assert!(is_silent(&vec![got[0]; 4000]));
        let loud: Vec<u8> = [0.5f32, -0.5]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let got = read_samples(&wav(3, 32, &loud)).unwrap();
        assert!(
            (got[0] - 16_383).abs() <= 1 && (got[1] + 16_383).abs() <= 1,
            "{got:?}"
        );
    }

    #[test]
    fn other_integer_widths_are_scaled_to_16_bit() {
        let s24 = [0x00u8, 0x00, 0x40, 0x00, 0x00, 0xc0]; // 0x400000, -0x400000
        assert_eq!(read_samples(&wav(1, 24, &s24)).unwrap(), [16_384, -16_384]);
        let s32: Vec<u8> = [0x4000_0000i32, -0x4000_0000]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        assert_eq!(read_samples(&wav(1, 32, &s32)).unwrap(), [16_384, -16_384]);
    }

    #[test]
    fn unsupported_wav_formats_are_an_error_not_garbage() {
        let err = read_samples(&wav(2, 4, &[0; 8])).unwrap_err().to_string();
        assert!(err.contains("unsupported WAV sample format"), "{err}");
        assert!(read_samples(&wav(1, 8, &[0; 8])).is_err());
    }

    #[test]
    fn a_killed_recorders_placeholder_size_still_reads() {
        let mut w = wav(1, 16, &[1, 0, 2, 0, 3, 0]);
        let at = w.len() - 6 - 4;
        w[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read_samples(&w).unwrap(), [1, 2, 3]);
    }

    /// Needs `whisper-cli`, a ggml model and a 16 kHz mono speech WAV (`COMEBACK_TEST_WAV`).
    #[cfg(feature = "whisper-tests")]
    #[test]
    fn transcribes_real_speech_with_word_times() {
        let wav = std::path::PathBuf::from(
            std::env::var("COMEBACK_TEST_WAV").expect("COMEBACK_TEST_WAV"),
        );
        let model = model_path("base.en")
            .or_else(|| model_path("small.en"))
            .expect("a ggml model");
        let scratch = tempfile::tempdir().unwrap();
        assert!(!is_silent(
            &read_samples(&std::fs::read(&wav).unwrap()).unwrap()
        ));
        let words = transcribe(&wav, scratch.path(), &model).unwrap();
        let text: Vec<_> = words.iter().map(|w| w.w.to_lowercase()).collect();
        assert!(
            text.contains(&"americans,".to_string()) && text.contains(&"fellow".to_string()),
            "{text:?}"
        );
        assert!(
            words.windows(2).all(|p| p[0].start_ms <= p[1].start_ms),
            "times rise"
        );
        assert!(words.last().unwrap().end_ms >= 5_000, "{words:?}");
    }
}
