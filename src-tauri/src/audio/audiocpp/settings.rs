//! Per-model option values, stored in the settings table.
//!
//! One JSON object per model, keyed by the model's path -- the same identity
//! the voice and recognition routers resolve -- inside one settings key. The
//! shape is deliberately loose: a model's options come from its spec, but a
//! spec can gain an option this app has not rendered yet, and the stored
//! record is what survives that.
//!
//! The server config reads the `load` side of what is stored, and the speech
//! request reads the `request` side; one record for both means nothing has to
//! move when a family grows from offline into streaming.

use serde_json::{Map, Value};

#[cfg(test)]
use super::specs::OptionSpec;
use crate::database::Database;

use super::specs::{FamilySpec, OptionKind};

/// The settings key, beside `app.tts` and `app.stt`.
const KEY: &str = "app.audio.options";

/// The request body a speech or transcription call carries for one model.
///
/// `None` when the user has never stored anything: the call then carries only
/// what it always did, so a plain voice stays plain.
pub fn request_options(database: &Database, path: &str, family: &str) -> Option<Value> {
    let stored = stored(database, path)?;
    let spec = super::specs::for_family(family);
    Some(request_values(spec.as_deref(), stored.as_object()))
}

/// The whole record: model path → one object of option values.
fn all(database: &Database) -> serde_json::Map<String, Value> {
    database
        .setting::<Value>(KEY)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

/// What the user has set for one model, if anything.
pub fn stored(database: &Database, path: &str) -> Option<Value> {
    all(database)
        .get(path)
        .filter(|value| value.is_object())
        .cloned()
}

/// Stores one model's options, whole.
///
/// The frontend saves one merged object rather than one field at a time, so a
/// retired option the record still carries is not silently resurrected.
pub fn save(database: &Database, path: &str, values: &Value) -> Result<(), String> {
    if !values.is_object() {
        return Err("Options are stored as an object of fields.".to_string());
    }
    let mut record = all(database);
    if values.as_object().is_some_and(Map::is_empty) {
        record.remove(path);
    } else {
        record.insert(path.to_string(), values.clone());
    }
    let write = Value::Object(record);
    database
        .set_setting(KEY, &write)
        .map_err(|error| format!("The options could not be stored: {error}"))
}

/// The request body keys one speech call carries: the spec's request options
/// the user set, plus anything else stored that is not a load option -- the
/// server is the one that knows whether an extra name is valid, so passing it
/// through is safer than hiding it.
pub fn request_values(spec: Option<&FamilySpec>, stored: Option<&Map<String, Value>>) -> Value {
    let mut body = Map::new();
    let Some(object) = stored else {
        return Value::Object(body);
    };
    let load_names: Vec<&str> = spec
        .map(|spec| {
            spec.load
                .iter()
                .map(|option| option.name.as_str())
                .chain(spec.session.iter().map(|option| option.name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    for (name, value) in object {
        if value.is_null() {
            continue;
        }
        if load_names.iter().any(|load| load == name) {
            continue;
        }
        body.insert(name.clone(), value.clone());
    }
    Value::Object(body)
}

/// The config's `load_options` one model takes: the spec's load options the
/// user set.
pub fn load_values(spec: Option<&FamilySpec>, stored: Option<&Map<String, Value>>) -> Value {
    let mut options = Map::new();
    let Some(spec) = spec else {
        return Value::Object(options);
    };
    let Some(stored) = stored else {
        return Value::Object(options);
    };
    for option in &spec.load {
        if let Some(value) = stored.get(&option.name).filter(|value| !value.is_null()) {
            if value_matches(&option.kind, value) {
                options.insert(option.name.clone(), value.clone());
            }
        }
    }
    Value::Object(options)
}

/// The user's stored values for one model's session options, keyed the way
/// the engine resolves them -- `{family}.{name}`.
///
/// The spec's session section lists *bare* names; the engine's session option
/// space is per family, which is exactly how a bare `espeak_library_path` got
/// parsed into the config and then ignored by the frontend. Stored values
/// also gate the config's own eSpeak auto-injection: a user's hand-set path
/// wins over the detected install, in both directions -- `None` means unset,
/// so the auto path is the only candidate.
pub fn session_values(
    spec: Option<&FamilySpec>,
    family: &str,
    stored: Option<&Map<String, Value>>,
) -> Map<String, Value> {
    let mut values = Map::new();
    let Some(spec) = spec else {
        return values;
    };
    for option in &spec.session {
        if let Some(value) = stored.and_then(|entry| entry.get(&option.name)) {
            values.insert(format!("{family}.{}", option.name), value.clone());
        }
    }
    values
}

/// Whether the spec's session section declares one named option, seen from
/// the config builder.
pub fn spec_takes_session(spec: Option<&FamilySpec>, name: &str) -> bool {
    spec.is_some_and(|spec: &FamilySpec| spec.has_session(name))
}

/// What a family has admitted to needing, learned from its own refusals.
///
/// A spec is *usually* the contract, but not always: Kokoro's spec names no
/// eSpeak session options while its runtime still refuses to speak without
/// them -- and, deeper, its frontend reads the eSpeak paths from the
/// *process environment* (`AUDIOCPP_ESPEAK_LIBRARY` / `_DATA`), not from
/// session options, which is why a session-keyed answer to its refusal gets
/// refused right back. So a trait has three shapes: `Session` (the family
/// takes the namespaced config keys), `Env` (the family wants the two
/// environment variables in the child process), `None` (neither). The runtime
/// negotiates -- a refusal says what the family actually takes, and the
/// answer is remembered here so the next config and spawn are right before
/// the server even starts.
const TRAITS_KEY: &str = "app.audio.family.traits";

/// The three eSpeak facts a family can learn, as the record stores them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EspeakTrait {
    /// The family takes `{family}.espeak_*` keys in its config.
    Session,
    /// The family reads the paths from the child process's environment.
    Env,
    /// The family takes no eSpeak wiring at all.
    None,
}

/// What the family has learned, from the recorded shape. `None` is unlearned
/// -- the spec is the only evidence so far.
pub fn espeak_trait(database: &Database, family: &str) -> Option<EspeakTrait> {
    let shape = database.setting::<Value>(TRAITS_KEY).and_then(|value| {
        value
            .get(family)
            .and_then(|record| record.get("espeak"))
            .cloned()
    })?;
    if let Some(needed) = shape.as_bool() {
        return Some(if needed {
            EspeakTrait::Session
        } else {
            EspeakTrait::None
        });
    }
    match shape.as_str() {
        Some("env") => Some(EspeakTrait::Env),
        Some("session") => Some(EspeakTrait::Session),
        Some(_) => Some(EspeakTrait::None),
        None => None,
    }
}

/// Records one negotiation outcome for a family.
fn learn(database: &Database, family: &str, shape: Value) -> Result<(), String> {
    let mut traits = database
        .setting::<Value>(TRAITS_KEY)
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    traits
        .entry(family.to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| "The family traits record is not an object.".to_string())?
        .insert("espeak".to_string(), shape);
    database
        .set_setting(TRAITS_KEY, &Value::Object(traits))
        .map_err(|error| format!("The learned family trait could not be stored: {error}"))
}

/// The family took the namespaced session keys, so the config carries them.
pub fn learn_session(database: &Database, family: &str) -> Result<(), String> {
    learn(database, family, Value::Bool(true))
}

/// The family wants the paths in the child process's environment instead --
/// how Kokoro's G2P reads them (`AUDIOCPP_ESPEAK_LIBRARY` / `_DATA`).
pub fn learn_env(database: &Database, family: &str) -> Result<(), String> {
    learn(database, family, Value::String("env".to_string()))
}

/// The family turned out to take no eSpeak wiring even though it had been
/// given some. Best effort: a failure here is not worth blocking speech over,
/// because the current call already carries the correction.
pub fn learn_env_clear(database: &Database, family: &str) {
    let _ = learn(database, family, Value::Bool(false));
}

/// Every family whose learned trait is `Env`, so the spawn knows whose
/// environment to write.
pub fn env_families(database: &Database) -> Vec<String> {
    let Some(traits) = database.setting::<Value>(TRAITS_KEY) else {
        return Vec::new();
    };
    let Some(traits) = traits.as_object() else {
        return Vec::new();
    };
    traits
        .iter()
        .filter(|(_, record)| record.get("espeak").and_then(Value::as_str) == Some("env"))
        .map(|(family, _)| family.clone())
        .collect()
}

/// Whether a stored value fits the type the spec names.
///
/// The config is what the server validates, so a wrong shape here is not a
/// crash -- but a string in an int field is silently dropped rather than
/// served, and this check makes that drop say nothing without shipping it.
fn value_matches(kind: &OptionKind, value: &Value) -> bool {
    match kind {
        OptionKind::Flag => value.is_boolean(),
        OptionKind::Integer => value.is_i64() || value.is_u64(),
        OptionKind::Number => value.is_number(),
        OptionKind::Text | OptionKind::Choice => value.is_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn spec_with_load(names: &[&str]) -> FamilySpec {
        let load = names
            .iter()
            .map(|name| OptionSpec {
                name: name.to_string(),
                // Text, because the test stores strings; the drop test below
                // covers what happens when the shape disagrees.
                kind: OptionKind::Text,
                description: String::new(),
                default: None,
                required: false,
                values: Vec::new(),
            })
            .collect();
        FamilySpec {
            display_name: "F".to_string(),
            tasks: Vec::new(),
            capabilities: BTreeMap::new(),
            request: Vec::new(),
            load,
            session: Vec::new(),
        }
    }

    /// A fixture whose session section names espeak's two options, the shape
    /// an eSpeak-backed family's spec actually carries.
    fn spec_with_session(names: &[&str]) -> FamilySpec {
        let session: Vec<OptionSpec> = names
            .iter()
            .map(|name| OptionSpec {
                name: name.to_string(),
                kind: OptionKind::Text,
                description: String::new(),
                default: None,
                required: false,
                values: Vec::new(),
            })
            .collect();
        FamilySpec {
            display_name: "F".to_string(),
            tasks: Vec::new(),
            capabilities: BTreeMap::new(),
            request: Vec::new(),
            load: Vec::new(),
            session,
        }
    }

    #[test]
    fn only_load_names_go_to_the_config() {
        let stored = serde_json::json!({
            "language": "en",
            "temperature": 0.9,
        });
        let map = stored.as_object().unwrap();
        let spec = spec_with_load(&["language"]);
        let load = load_values(Some(&spec), Some(map));
        assert_eq!(load["language"], "en");
        assert!(load.get("temperature").is_none());

        let request = request_values(Some(&spec), Some(map));
        assert_eq!(request["temperature"], 0.9);
        assert!(request.get("language").is_none());
    }

    #[test]
    fn a_wrong_shaped_load_value_is_dropped_rather_than_served() {
        let stored = serde_json::json!({ "language": 42 });
        let spec = spec_with_load(&["language"]);
        let load = load_values(Some(&spec), Some(stored.as_object().unwrap()));
        assert!(load.as_object().unwrap().is_empty());
    }

    #[test]
    fn session_keys_carry_the_family_namespace() {
        // The bare names leave the record exactly as the user saw them on the
        // page; the config's key is what the engine resolves -- the namespace
        // the engine reads `inflect_v2.espeak_data_path` under.
        let spec = spec_with_session(&["espeak_library_path", "espeak_data_path"]);
        let stored = serde_json::json!({ "espeak_library_path": "C:\\espeak\\espeak-ng.dll" });
        let values = session_values(Some(&spec), "sanotts", Some(stored.as_object().unwrap()));
        assert_eq!(
            values["sanotts.espeak_library_path"],
            "C:\\espeak\\espeak-ng.dll"
        );
        assert!(values.get("sanotts.espeak_data_path").is_none());
    }

    #[test]
    fn a_family_without_session_options_carries_none() {
        // This is what stopped the eSpeak paths landing on a Kitten model and
        // the server refusing an "unknown" option: no declared option, no key.
        let spec = spec_with_session(&[]);
        let values = session_values(Some(&spec), "kitten_tts2", None);
        assert!(values.is_empty());
    }
}
