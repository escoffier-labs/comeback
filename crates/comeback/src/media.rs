//! Reading the recorded MP4 back with `ffprobe` and `ffmpeg`.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use comeback_core::grid::LumaGrid;
use comeback_core::keyframes::Candidate;
use image::RgbImage;

/// Candidate frames per second, matching the spec's 4 fps branch.
const CANDIDATE_FPS: u64 = 4;
/// Candidate frame width; the luma grid only needs a coarse picture.
const CANDIDATE_WIDTH: u32 = 480;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
}

fn tool(name: &str) -> String {
    let var = format!("COMEBACK_{}", name.to_uppercase());
    std::env::var(var).unwrap_or_else(|_| name.to_string())
}

pub fn probe(video: &Path) -> Result<VideoInfo> {
    let out = Command::new(tool("ffprobe"))
        .args(["-v", "error", "-select_streams", "v:0"])
        .args([
            "-show_entries",
            "stream=width,height,duration:format=duration",
            "-of",
            "json",
        ])
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .context("running ffprobe (is ffmpeg installed?)")?;
    if !out.status.success() {
        bail!(
            "ffprobe could not read {}: {}",
            video.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).context("ffprobe output")?;
    let stream = &v["streams"][0];
    let (Some(width), Some(height)) = (stream["width"].as_u64(), stream["height"].as_u64()) else {
        bail!("{} has no video stream", video.display());
    };
    // The video stream's own length, not the container's: the audio track runs on after the last
    // screen frame (by up to a tenth of a second), and a frame can only be cut from the video.
    let secs: f64 = [&stream["duration"], &v["format"]["duration"]]
        .into_iter()
        .filter_map(|d| d.as_str()?.parse::<f64>().ok())
        .find(|d| *d > 0.0)
        .unwrap_or(0.0);
    Ok(VideoInfo {
        width: width as u32,
        height: height as u32,
        duration_ms: (secs * 1000.0).round() as u64,
    })
}

/// Size of the downscaled candidate frames for a video: 480 wide (never upscaled), even height.
pub fn candidate_size(width: u32, height: u32) -> (u32, u32) {
    let w = width.clamp(2, CANDIDATE_WIDTH) & !1;
    let h = ((u64::from(height) * u64::from(w) / u64::from(width.max(1))) as u32).max(2) & !1;
    (w, h)
}

/// Luma grids of the video sampled at 4 fps.
pub fn candidates(video: &Path, info: &VideoInfo) -> Result<Vec<Candidate>> {
    let (w, h) = candidate_size(info.width, info.height);
    let mut child = Command::new(tool("ffmpeg"))
        .args(["-v", "error", "-threads", "1", "-i"])
        .arg(video)
        .args(["-vf", &format!("fps={CANDIDATE_FPS},scale={w}:{h}")])
        .args(["-f", "rawvideo", "-pix_fmt", "rgb24", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running ffmpeg (is ffmpeg installed?)")?;
    let mut stdout = child.stdout.take().context("ffmpeg stdout")?;
    let frame_len = (w * h * 3) as usize;
    let mut buf = vec![0u8; frame_len];
    let mut out = Vec::new();
    while read_full(&mut stdout, &mut buf)? {
        let img = RgbImage::from_raw(w, h, buf.clone()).context("candidate frame size")?;
        out.push(Candidate {
            at_ms: out.len() as u64 * 1000 / CANDIDATE_FPS,
            grid: LumaGrid::from_image(&img),
        });
    }
    let status = child.wait()?;
    if !status.success() && out.is_empty() {
        let mut err = String::new();
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
        bail!(
            "ffmpeg could not decode {}: {}",
            video.display(),
            err.trim()
        );
    }
    Ok(out)
}

/// Fills `buf` completely; `false` on a clean end of stream before the first byte.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> Result<bool> {
    let mut got = 0;
    while got < buf.len() {
        let n = r.read(&mut buf[got..])?;
        if n == 0 {
            return Ok(false);
        }
        got += n;
    }
    Ok(true)
}

/// How far back from the requested time to try again when nothing can be decoded there.
const FRAME_RETRY_BACK_MS: [u64; 5] = [0, 100, 250, 500, 1000];

/// One full-size frame at `at_ms`, or the nearest earlier one: a request at the very end of the
/// recording can fall just past the last frame the encoder wrote.
pub fn frame_near(video: &Path, at_ms: u64) -> Result<RgbImage> {
    let mut last = None;
    for back in FRAME_RETRY_BACK_MS {
        match frame_at(video, at_ms.saturating_sub(back)) {
            Ok(img) => return Ok(img),
            Err(e) => last = Some(e),
        }
        if at_ms < back {
            break;
        }
    }
    Err(last.expect("at least one attempt"))
}

/// One full-size frame at exactly `at_ms`.
pub fn frame_at(video: &Path, at_ms: u64) -> Result<RgbImage> {
    let out = Command::new(tool("ffmpeg"))
        .args(["-v", "error", "-threads", "1", "-ss"])
        .arg(format!("{}.{:03}", at_ms / 1000, at_ms % 1000))
        .arg("-i")
        .arg(video)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .stdin(Stdio::null())
        .output()
        .context("running ffmpeg (is ffmpeg installed?)")?;
    if out.stdout.is_empty() {
        bail!(
            "no frame at {at_ms} ms in {}: {}",
            video.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(image::load_from_memory(&out.stdout)
        .context("decoding frame")?
        .to_rgb8())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A recording whose audio outlasts its video, as the encoder produces: 1.0 s of picture and
    /// 1.5 s of sound in one MP4.
    fn audio_outlasts_video(dir: &Path) -> Option<std::path::PathBuf> {
        let ok = Command::new(tool("ffmpeg")).arg("-version").output().ok()?;
        if !ok.status.success() {
            return None;
        }
        let path = dir.join("long-audio.mp4");
        let made = Command::new(tool("ffmpeg"))
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=blue:s=64x64:d=1:r=10",
            ])
            .args(["-f", "lavfi", "-i", "sine=d=1.5"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .arg(&path)
            .status()
            .ok()?;
        made.success().then_some(path)
    }

    #[test]
    fn duration_is_the_videos_not_the_audios_and_the_last_frame_is_reachable() {
        let dir = tempfile::tempdir().unwrap();
        let Some(mp4) = audio_outlasts_video(dir.path()) else {
            assert_ne!(
                std::env::var("COMEBACK_REQUIRE_X11_TESTS").as_deref(),
                Ok("1"),
                "ffmpeg is required"
            );
            eprintln!("SKIP: ffmpeg is not installed");
            return;
        };
        let info = probe(&mp4).unwrap();
        assert!(
            (900..=1100).contains(&info.duration_ms),
            "the container says 1500 ms, the picture ends at 1000: {info:?}"
        );
        assert_eq!((info.width, info.height), (64, 64));
        // The container's length is past the last frame: the plain seek finds nothing there ...
        assert!(frame_at(&mp4, 1450).is_err());
        // ... and the tolerant one steps back to the last frame instead of failing the compile.
        let img = frame_near(&mp4, 1450).unwrap();
        assert_eq!(img.dimensions(), (64, 64));
        assert!(img.get_pixel(5, 5).0[2] > 150, "it is the blue picture");
    }

    #[test]
    fn candidate_frames_are_480_wide_with_even_sides() {
        assert_eq!(candidate_size(1920, 1080), (480, 270));
        assert_eq!(candidate_size(2560, 1440), (480, 270));
        assert_eq!(candidate_size(640, 360), (480, 270));
        assert_eq!(candidate_size(300, 200), (300, 200), "never upscaled");
        assert_eq!(candidate_size(1000, 333), (480, 158));
    }
}
