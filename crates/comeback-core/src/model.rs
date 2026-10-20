use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub id: String,
    pub started_at: String,
    pub duration_ms: u64,
    pub backend: String,
    pub target: Target,
    pub keyframes: Vec<Keyframe>,
    #[serde(default)]
    pub marks: Vec<Mark>,
    #[serde(default)]
    pub events: Vec<Event>,
    #[serde(default)]
    pub page: Vec<PageEvent>,
    pub transcript: Option<Transcript>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    Screen,
    Window,
    Area,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub kind: TargetKind,
    /// x, y, width, height in physical pixels, global coordinates.
    pub rect: [i32; 4],
    pub scale: f64,
    pub app: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Reason {
    Mark,
    Click,
    Drag,
    Cue,
    Scene,
    Midpoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Keyframe {
    pub n: u32,
    pub at_ms: u64,
    pub reason: Reason,
    pub file: String,
    pub said: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub id: u32,
    pub at_ms: u64,
    pub end_ms: u64,
    /// x0, y0, x1, y1 as fractions of the target.
    pub bbox: [f64; 4],
    /// Each stroke is a list of [x, y, t_ms]; x and y are fractions.
    pub strokes: Vec<Vec<[f64; 3]>>,
    pub keyframe: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Click,
    Drag,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub at_ms: u64,
    pub kind: EventKind,
    pub x: f64,
    pub y: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<[f64; 2]>,
    pub button: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageEvent {
    pub at_ms: u64,
    pub kind: String,
    pub url: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transcript {
    pub model: String,
    pub words: Vec<Word>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub w: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn sample() -> Manifest {
        Manifest {
            schema: SCHEMA,
            id: "comeback-1".into(),
            started_at: "2026-10-19T00:00:00Z".into(),
            duration_ms: 33_000,
            backend: "x11".into(),
            target: Target {
                kind: TargetKind::Screen,
                rect: [0, 0, 2560, 1440],
                scale: 1.0,
                app: Some("firefox".into()),
                title: Some("Settings".into()),
            },
            keyframes: vec![Keyframe {
                n: 1,
                at_ms: 4_000,
                reason: Reason::Click,
                file: "frames/frame-01.png".into(),
                said: "hi".into(),
                action: Some("clicked".into()),
            }],
            marks: vec![],
            events: vec![Event {
                at_ms: 4_000,
                kind: EventKind::Click,
                x: 0.5,
                y: 0.5,
                to: None,
                button: Some("left".into()),
                label: None,
            }],
            page: vec![],
            transcript: Some(Transcript {
                model: "small.en".into(),
                words: vec![Word {
                    w: "hi".into(),
                    start_ms: 4_000,
                    end_ms: 4_200,
                }],
            }),
            notes: vec![],
        }
    }

    #[test]
    fn round_trips_with_camel_case_keys() {
        let m = sample();
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"durationMs\":33000"));
        assert!(json.contains("\"startMs\":4000"));
        assert!(json.contains("\"reason\":\"click\""));
        assert!(!json.contains("\"notes\""), "empty notes are omitted");
        let back: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
    }
}
