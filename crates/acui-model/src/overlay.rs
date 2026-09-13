// SPDX-License-Identifier: AGPL-3.0-only
//! Geometry pulled out of an opaque payload, defensively: whatever looks like a
//! rectangle or a point is drawn, everything else is ignored.

use serde_json::Value;

/// A rectangle in frame coordinates; a point is a rectangle of zero size.
#[derive(Debug, Clone, PartialEq)]
pub struct Overlay {
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Overlay {
    pub fn is_point(&self) -> bool {
        self.width <= 0.0 && self.height <= 0.0
    }
}

pub fn extract_overlays(payload: &Value) -> Vec<Overlay> {
    let mut overlays = Vec::new();
    walk(payload, "payload", &mut overlays);
    overlays
}

/// Frame size when the payload states one, searched by `frame_width`/`frame_height`.
pub fn extract_frame_size(payload: &Value) -> Option<(f32, f32)> {
    match payload {
        Value::Object(map) => {
            let size = number(map.get("frame_width")).zip(number(map.get("frame_height")));
            size.or_else(|| map.values().find_map(extract_frame_size))
        }
        Value::Array(items) => items.iter().find_map(extract_frame_size),
        _ => None,
    }
}

fn walk(value: &Value, name: &str, overlays: &mut Vec<Overlay>) {
    match value {
        Value::Object(map) => {
            let x = number(map.get("x"));
            let y = number(map.get("y"));
            if let (Some(x), Some(y)) = (x, y) {
                overlays.push(Overlay {
                    kind: name.to_string(),
                    x,
                    y,
                    width: number(map.get("width")).unwrap_or(0.0),
                    height: number(map.get("height")).unwrap_or(0.0),
                });
            }
            for index in 1..=3 {
                let x = number(map.get(&format!("x{index}")));
                let y = number(map.get(&format!("y{index}")));
                if let (Some(x), Some(y)) = (x, y) {
                    overlays.push(Overlay {
                        kind: format!("{name}.{index}"),
                        x,
                        y,
                        width: 0.0,
                        height: 0.0,
                    });
                }
            }
            for (key, child) in map {
                walk(child, key, overlays);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk(child, &format!("{name}[{index}]"), overlays);
            }
        }
        _ => {}
    }
}

fn number(value: Option<&Value>) -> Option<f32> {
    value.and_then(Value::as_f64).map(|value| value as f32)
}
