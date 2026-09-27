//! Detects when the foreground video plays through the secure video path (SVP).
//!
//! DRM-protected native apps (Netflix, Disney+, Prime Video, ...) decode into secure memory,
//! and the webOS VT driver deliberately refuses to capture that path ("Vdec source security is
//! TRUE, could not do vt capture!"). This module only *detects* that state so the daemon can stop
//! retrying capture, keep the kernel log quiet and hand the lights back to their own mode.
//! It does not attempt to capture protected content.

use serde_json::Value;
use std::process::Command;

const PIPELINES_URI: &str = "luna://com.webos.media/getActivePipelines";

/// Returns `Some(true)` while a foreground media pipeline holds secure-video resources,
/// `Some(false)` when none does, and `None` when the state cannot be read (e.g. off-TV).
pub fn secure_playback_active() -> Option<bool> {
    if !std::path::Path::new("/usr/bin/luna-send").exists() {
        return None;
    }
    let output = Command::new("/usr/bin/luna-send")
        .args(["-n", "1", "-w", "1000", PIPELINES_URI, "{}"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value: Value = serde_json::from_slice(&output.stdout).ok()?;
    pipelines_indicate_secure(&value)
}

/// Pure parser for `getActivePipelines` responses (a bare array, or an object wrapping one).
pub fn pipelines_indicate_secure(value: &Value) -> Option<bool> {
    let pipelines = match value {
        Value::Array(items) => items,
        Value::Object(fields) => {
            if fields.get("returnValue") == Some(&Value::Bool(false)) {
                return None;
            }
            fields.values().find_map(Value::as_array)?
        }
        _ => return None,
    };

    Some(pipelines.iter().any(|pipeline| {
        let foreground = pipeline
            .get("is_foreground")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let is_media = pipeline.get("type").and_then(Value::as_str) == Some("media");
        let secure = pipeline
            .get("resource")
            .and_then(Value::as_array)
            .map(|resources| {
                resources.iter().any(|r| {
                    r.get("resource")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.starts_with("SVP"))
                })
            })
            .unwrap_or(false);
        foreground && is_media && secure
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn foreground_media_pipeline_with_svp_resources_is_secure() {
        // Trimmed from a real OLED55BX6LB response while Disney+ was playing.
        let response = json!([
            {"type": "avconnector", "is_foreground": true,
             "resource": [{"resource": "MAIN_SCALER", "index": 0}]},
            {"type": "tv", "is_foreground": false,
             "resource": [{"resource": "HDMI_INPUT", "index": 2}]},
            {"type": "media", "is_foreground": true,
             "resource": [{"resource": "SVP_CPB", "index": 1}, {"resource": "VDEC", "index": 0}]}
        ]);
        assert_eq!(pipelines_indicate_secure(&response), Some(true));
    }

    #[test]
    fn clear_or_background_playback_is_not_secure() {
        let clear = json!([{"type": "media", "is_foreground": true,
            "resource": [{"resource": "VDEC", "index": 0}]}]);
        assert_eq!(pipelines_indicate_secure(&clear), Some(false));

        let background = json!([{"type": "media", "is_foreground": false,
            "resource": [{"resource": "SVP_CPB", "index": 0}]}]);
        assert_eq!(pipelines_indicate_secure(&background), Some(false));

        assert_eq!(pipelines_indicate_secure(&json!([])), Some(false));
    }

    #[test]
    fn failed_or_malformed_responses_are_unknown() {
        assert_eq!(
            pipelines_indicate_secure(&json!({"returnValue": false, "errorText": "denied"})),
            None
        );
        assert_eq!(pipelines_indicate_secure(&json!("nope")), None);
    }
}
