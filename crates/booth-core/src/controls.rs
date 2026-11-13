//! Friendly voice controls on top of the steering bundle (`GET /v2/controls`, its `ui` part).
//!
//! People (and Claude) write controls the way the ZONOS3 studio's presets do:
//! - a control id: `"age": 1.5`, `"deep": 2`, `"happy": 4`, `"pace": -0.8`
//! - an accent: `"accent:british": 1.5`
//! - or a raw direction: `"identity.gender": -1`
//!
//! `expand` turns them into the raw `"<set>.<direction>" -> dose` map the engine takes.

use std::collections::BTreeMap;

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub id: String,
    pub group: String,
    pub label: String,
    /// `bipolar` (negative and positive poles) or `unipolar` (0 and up).
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub poles: Option<Vec<String>>,
    /// `"-"` / `"+"` → direction doses per unit.
    #[serde(default)]
    pub map: BTreeMap<String, BTreeMap<String, f64>>,
    #[serde(default)]
    pub range: Option<[f64; 2]>,
    /// Doses inside this range keep the voice recognisable.
    #[serde(default)]
    pub safe: Option<[f64; 2]>,
    #[serde(default)]
    pub effect: Option<String>,
    /// Works only in Shapeable mode (moves the speaker embedding).
    #[serde(default)]
    pub shapeable_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Accent {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub tab: Option<String>,
    #[serde(default)]
    pub map: BTreeMap<String, f64>,
    #[serde(default)]
    pub range: Option<[f64; 2]>,
    #[serde(default)]
    pub safe: Option<[f64; 2]>,
    #[serde(default)]
    pub effect: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub label: String,
}

/// The parts of the controls document Booth uses.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Catalog {
    pub groups: Vec<Group>,
    pub controls: Vec<Control>,
    pub accents: Vec<Accent>,
    /// Named looks, e.g. "BBC newsreader" → {"accent:british": 1.5, "authority": 3, "room": 2}.
    pub presets: BTreeMap<String, BTreeMap<String, f64>>,
    pub preview_text: Option<String>,
    /// Every raw direction id (`<set>.<direction>`).
    pub directions: Vec<String>,
    /// Sets whose directions need Shapeable mode.
    pub raw_sets: Vec<String>,
}

/// The server sends some UTF-8 text encoded twice ("Â±2"); undo that when it round-trips.
fn fix_text(s: &str) -> String {
    if s.chars().all(|c| (c as u32) < 256) {
        let bytes: Vec<u8> = s.chars().map(|c| c as u8).collect();
        if let Ok(fixed) = String::from_utf8(bytes) {
            return fixed;
        }
    }
    s.to_string()
}

impl Catalog {
    pub fn from_doc(doc: &Value) -> Result<Self> {
        let ui = doc.get("ui").cloned().unwrap_or(Value::Null);
        let raw_sets: Vec<String> = doc
            .get("sets")
            .and_then(|s| s.as_object())
            .map(|m| {
                m.iter()
                    .filter(|(_, v)| v.get("space").and_then(|s| s.as_str()) == Some("raw"))
                    .map(|(k, _)| k.clone())
                    .collect()
            })
            .unwrap_or_default();
        let directions: Vec<String> = doc
            .get("directions")
            .and_then(|d| d.as_object())
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        let parse = |k: &str| ui.get(k).cloned().unwrap_or(Value::Null);
        let groups: Vec<Group> = serde_json::from_value(parse("groups")).unwrap_or_default();
        let mut controls: Vec<Control> = serde_json::from_value(parse("controls")).unwrap_or_default();
        let mut accents: Vec<Accent> = serde_json::from_value(parse("accents")).unwrap_or_default();
        let presets = serde_json::from_value(parse("presets")).unwrap_or_default();
        let preview_text = parse("preview_text").as_str().map(fix_text);
        let is_raw = |dir: &str| raw_sets.iter().any(|s| dir.split('.').next() == Some(s.as_str()));
        for c in &mut controls {
            c.effect = c.effect.as_deref().map(fix_text);
            c.shapeable_only = c.map.values().flat_map(|m| m.keys()).any(|d| is_raw(d));
        }
        for a in &mut accents {
            a.effect = a.effect.as_deref().map(fix_text);
        }
        Ok(Self { groups, controls, accents, presets, preview_text, directions, raw_sets })
    }

    pub fn needs_shapeable(&self, direction: &str) -> bool {
        self.raw_sets.iter().any(|s| direction.split('.').next() == Some(s.as_str()))
    }

    /// Friendly controls → raw direction doses (summed when two controls share a direction).
    pub fn expand(&self, friendly: &BTreeMap<String, f64>) -> Result<BTreeMap<String, f64>> {
        let mut out: BTreeMap<String, f64> = BTreeMap::new();
        let mut add = |dir: &str, dose: f64| *out.entry(dir.to_string()).or_insert(0.0) += dose;
        for (key, &dose) in friendly {
            if dose == 0.0 {
                continue;
            }
            let k = key.trim();
            if let Some(acc) = k.strip_prefix("accent:") {
                let a = self
                    .accents
                    .iter()
                    .find(|a| a.id.eq_ignore_ascii_case(acc))
                    .ok_or_else(|| anyhow::anyhow!("no accent '{acc}'"))?;
                for (dir, unit) in &a.map {
                    add(dir, unit * dose);
                }
            } else if let Some(c) = self.controls.iter().find(|c| c.id.eq_ignore_ascii_case(k)) {
                let pole = if dose < 0.0 { "-" } else { "+" };
                let Some(m) = c.map.get(pole) else {
                    bail!("{} only goes up (0 and above)", c.label);
                };
                for (dir, unit) in m {
                    add(dir, unit * dose.abs());
                }
            } else if self.directions.iter().any(|d| d == k) || (self.directions.is_empty() && k.contains('.')) {
                add(k, dose);
            } else {
                bail!("unknown control '{k}' (list_controls shows the names)");
            }
        }
        Ok(out)
    }

    /// A compact description for Claude: groups → controls, accents, presets.
    pub fn summary(&self) -> Value {
        let groups: Vec<Value> = self
            .groups
            .iter()
            .map(|g| {
                let cs: Vec<Value> = self
                    .controls
                    .iter()
                    .filter(|c| c.group == g.id)
                    .map(|c| {
                        serde_json::json!({
                            "id": c.id,
                            "label": c.label,
                            "poles": c.poles,
                            "range": c.range,
                            "safe": c.safe,
                            "shapeable_only": c.shapeable_only,
                            "effect": c.effect,
                        })
                    })
                    .collect();
                serde_json::json!({ "group": g.label, "controls": cs })
            })
            .filter(|g| g["controls"].as_array().is_some_and(|a| !a.is_empty()))
            .collect();
        let accents: Vec<Value> = self
            .accents
            .iter()
            .map(|a| serde_json::json!({ "id": format!("accent:{}", a.id), "label": a.label, "safe": a.safe }))
            .collect();
        serde_json::json!({
            "how": "Pass controls as {\"<id>\": dose}. Bipolar controls go negative toward the first pole and positive toward the second; unipolar ones go from 0 up. Stay inside 'safe' to keep the voice recognisable. Accents and shapeable_only controls need a Shapeable voice or mode \"shapeable\".",
            "groups": groups,
            "accents": accents,
            "presets": self.presets,
        })
    }
}
