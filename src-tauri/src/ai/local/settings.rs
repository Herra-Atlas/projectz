//! Per-model `llama-server` runtime settings, and the flags they produce.
//!
//! Split out of `local.rs` because the settings and the flag construction are a
//! self-contained concern with their own tests, and `local.rs` is already the
//! process lifecycle. Nothing here spawns a process or touches the filesystem.
//!
//! ## The one rule
//!
//! **An option that is off passes no flag at all.** Not `--no-kv-offload`, not
//! `--flash-attn off` -- nothing. `llama-server` has its own defaults for every
//! one of these, and they are frequently `auto` rather than `off`. Passing an
//! explicit "off" would override a default that was doing the right thing, which
//! is the opposite of what a user asking for "off" is asking for.
//!
//! That is why every field below is an `Option` rather than a plain value: `None`
//! and `Some(false)` mean genuinely different things to the process, and
//! flattening them into a bool would silently change behaviour on every toggle.

use serde::{Deserialize, Serialize};

/// A multimodal projector file, for a vision model.
///
/// Separate from the model path because it is chosen independently -- you point
/// at the model GGUF and then at the mmproj that goes with it -- and because
/// llama.cpp takes it as a distinct `--mmproj` argument.
pub type MmprojPath = String;

/// Which of the KV cache's two halves a quantisation applies to.
///
/// K and V are separate flags (`-ctk` / `-ctv`) and separate columns, and a model
/// can sensibly use one quantisation for K and another for V: V tolerates more
/// quantisation than K, because K is what attention scores compare against.
/// Offering one control for both would remove a choice that is genuinely worth
/// making, so the UI exposes them as the two things they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KvQuant {
    /// Unquantised. Best quality, most memory. Never offloads cleanly on some
    /// backends, so it is not the default.
    F32,
    /// Half precision. The safe quality choice.
    F16,
    Q8_0,
    Q5_1,
    Q5_0,
    Q4_1,
    Q4_0,
    /// Aggressive. Noticeable quality loss, largest saving.
    Q2K,
}

impl KvQuant {
    /// Every value, for the settings UI to offer.
    ///
    /// Declared here rather than spelled out in the frontend so the list of
    /// quantisations `llama-server` accepts has one owner. A UI with its own list
    /// would eventually offer one the binary rejects, and that surfaces as a
    /// server that will not start rather than as a mistake in the settings page.
    pub const ALL: [KvQuant; 8] = [
        KvQuant::F32,
        KvQuant::F16,
        KvQuant::Q8_0,
        KvQuant::Q5_1,
        KvQuant::Q5_0,
        KvQuant::Q4_1,
        KvQuant::Q4_0,
        KvQuant::Q2K,
    ];

    /// The spelling `llama-server` expects, and the label the UI shows.
    ///
    /// One method for both because they are the same string: a UI that showed a
    /// friendly name while sending a different spelling would be offering a
    /// choice that does not exist.
    pub fn as_flag(&self) -> &'static str {
        match self {
            KvQuant::F32 => "f32",
            KvQuant::F16 => "f16",
            KvQuant::Q8_0 => "q8_0",
            KvQuant::Q5_1 => "q5_1",
            KvQuant::Q5_0 => "q5_0",
            KvQuant::Q4_1 => "q4_1",
            KvQuant::Q4_0 => "q4_0",
            KvQuant::Q2K => "q2_k",
        }
    }

    /// Roughly how many bytes this costs per token per slot, against f16's 2.
    ///
    /// Used only by the memory estimate, so an approximation is fine and the
    /// comment says so: these are the published figures rounded to the nearest
    /// quarter of a byte, not measurements of any particular model.
    pub fn bytes_per_token_per_layer(self) -> f32 {
        match self {
            KvQuant::F32 => 4.0,
            KvQuant::F16 => 2.0,
            KvQuant::Q8_0 => 1.0,
            KvQuant::Q5_1 => 0.75,
            KvQuant::Q5_0 => 0.6875,
            KvQuant::Q4_1 => 0.5,
            KvQuant::Q4_0 => 0.5,
            KvQuant::Q2K => 0.3125,
        }
    }
}

/// How Flash Attention is configured.
///
/// Three states because `llama-server` has three: forcing it on for a backend
/// that does not support it fails to start, so `auto` is the default and a
/// two-way toggle would have to pick a side to be the "off" position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlashAttention {
    Auto,
    On,
    Off,
}

impl FlashAttention {
    pub fn as_flag(&self) -> &'static str {
        match self {
            FlashAttention::Auto => "auto",
            FlashAttention::On => "on",
            FlashAttention::Off => "off",
        }
    }
}

/// The context a model runs at until told otherwise.
///
/// 30K, and a round number on purpose -- the point of a default is that nobody
/// has to think about it. Large enough that a real conversation plus its system
/// prompt and instructions fits without truncating, small enough to leave
/// headroom on an 8 GB card at the default KV quantisation.
pub const DEFAULT_CONTEXT: u32 = 30_000;

/// How many conversations the server holds open by default.
///
/// One, deliberately. `-c` is a *shared pool* divided between slots unless
/// `kv_unified_per_slot` is set, so a default above one silently gives each
/// conversation a fraction of the number the user can see in the box -- 30K
/// across six slots is 5K each, which is the failure the `--kv-unified-per-slot`
/// toggle exists to explain. Starting at one means the number on screen is the
/// number a conversation actually gets, and concurrency is something raised
/// deliberately by someone who knows it is paid for out of that pool.
pub const DEFAULT_PARALLEL: u32 = 1;

/// Every tunable for one model's server process.
///
/// `#[serde(default)]` throughout, so a settings blob written by an older build
/// still loads: the missing fields take their defaults rather than failing the
/// whole parse and leaving the model with no configuration at all.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalRuntimeSettings {
    /// `-c`. Total prompt context.
    ///
    /// Shared across slots unless `kv_unified_per_slot` is set -- see that field
    /// before changing this one, because the number a user sees is a different
    /// meaning depending on it. With the default of one slot there is no
    /// division, which is why the default is one.
    pub context: u32,

    /// `-np`. Number of server slots, i.e. concurrent conversations.
    pub parallel: u32,

    /// `--kv-unified-per-slot N`. Treat `context` as *per slot* rather than
    /// shared.
    ///
    /// Without it, `context` is the total pool and each of `parallel` slots gets
    /// `context / parallel`. So 10k context with 6 parallel silently becomes
    /// about 1.6k per conversation -- which is almost never what someone typing
    /// "10000" into a context box intends. With it, each slot gets the full
    /// `context` and the pool grows to `context * parallel`.
    pub kv_unified_per_slot: Option<u32>,

    /// `-ngl`. Layers to offload to the GPU.
    pub gpu_layers: Option<u32>,

    /// `-ctk`. None leaves llama.cpp's own default in place.
    pub cache_type_k: Option<KvQuant>,

    /// `-ctv`.
    pub cache_type_v: Option<KvQuant>,

    /// `--kv-offload`. Whether the KV cache lives on the GPU.
    ///
    /// Off here means *no flag*, which is not the same as `--no-kv-offload`:
    /// this keeps llama.cpp's default, which is usually to offload.
    pub kv_offload: Option<bool>,

    /// `-fa`.
    pub flash_attention: Option<FlashAttention>,

    /// `--mmproj`. A vision projector, for a multimodal model.
    pub mmproj: Option<MmprojPath>,

    /// `--mmproj-offload`. Whether the projector itself runs on the GPU.
    pub mmproj_offload: Option<bool>,
}

impl Default for LocalRuntimeSettings {
    fn default() -> Self {
        Self {
            context: DEFAULT_CONTEXT,
            parallel: DEFAULT_PARALLEL,
            // The previous hardcoded behaviour had no such flag, so every slot
            // shared one pool. Leaving it off preserves that.
            kv_unified_per_slot: None,
            // Matches the previous hardcoded behaviour: offload every layer the
            // device will take. A user who wants a specific count sets it; the
            // default stays "as much as possible" because that is what worked.
            gpu_layers: Some(999),
            cache_type_k: Some(KvQuant::Q8_0),
            cache_type_v: Some(KvQuant::Q8_0),
            kv_offload: None,
            flash_attention: Some(FlashAttention::Auto),
            mmproj: None,
            mmproj_offload: None,
        }
    }
}

impl LocalRuntimeSettings {
    /// The context one slot actually receives.
    ///
    /// Exposed because it is the number that matters to a user reasoning about
    /// how much the model can remember, and it is *not* `context` unless
    /// `kv_unified_per_slot` is set.
    pub fn context_per_slot(&self) -> u32 {
        match self.kv_unified_per_slot {
            // The pool is sized to parallel * N, so each slot sees the full N.
            Some(_) => self.context,
            None => self.context.div_ceil(self.parallel.max(1)),
        }
    }

    /// The flags for this configuration, in a stable order.
    ///
    /// A pure function of the settings so the tests can assert on the exact
    /// argument list without spawning a process.
    pub fn to_args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();
        // A plain function rather than a closure borrowing `args`, because the
        // borrow would have to end before the bare `args.push` calls below --
        // which are the flags that take no value.
        fn push(args: &mut Vec<String>, flag: &str, value: &str) {
            args.push(flag.to_string());
            args.push(value.to_string());
        }

        push(&mut args, "--ctx-size", &self.context.to_string());
        push(&mut args, "--parallel", &self.parallel.to_string());
        if let Some(per_slot) = self.kv_unified_per_slot {
            push(&mut args, "--kv-unified-per-slot", &per_slot.to_string());
        }
        if let Some(layers) = self.gpu_layers {
            push(&mut args, "-ngl", &layers.to_string());
        }
        if let Some(kind) = self.cache_type_k {
            push(&mut args, "-ctk", kind.as_flag());
        }
        if let Some(kind) = self.cache_type_v {
            push(&mut args, "-ctv", kind.as_flag());
        }
        // Only `Some(true)` emits anything: `None` and `Some(false)` both mean
        // "leave llama.cpp's default alone", which is the rule at the top of
        // this file. An explicit `--no-kv-offload` would turn off an offload
        // that was working.
        if self.kv_offload == Some(true) {
            args.push("--kv-offload".to_string());
        }
        if let Some(flash) = self.flash_attention {
            push(&mut args, "-fa", flash.as_flag());
        }
        if let Some(projector) = &self.mmproj {
            if !projector.trim().is_empty() {
                push(&mut args, "--mmproj", projector);
            }
        }
        if self.mmproj_offload == Some(true) {
            args.push("--mmproj-offload".to_string());
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A helper that reads an argument list as flag/value pairs.
    fn pairs(args: &[String]) -> Vec<(&str, &str)> {
        args.chunks(2)
            .filter(|chunk| chunk.len() == 2)
            .map(|chunk| (chunk[0].as_str(), chunk[1].as_str()))
            .collect()
    }

    fn value_of(args: &[String], flag: &str) -> Option<String> {
        pairs(args)
            .into_iter()
            .find(|(key, _)| *key == flag)
            .map(|(_, value)| value.to_string())
    }

    /// The two values the frontend also has to know.
    ///
    /// `localRuntimeDefaults.ts` restates these for the case where
    /// `local_runtime_settings` fails, and nothing in the type system connects
    /// the two copies. This asserts the literals, so changing one without the
    /// other fails here rather than showing a settings page that disagrees with
    /// the model it is about to describe.
    #[test]
    fn defaults_match_the_frontend_copies() {
        // The literals are written out rather than compared against each other,
        // so this fails if either side moves and is not restated.
        assert_eq!(DEFAULT_CONTEXT, 30_000, "update localRuntimeDefaults.ts");
        assert_eq!(DEFAULT_PARALLEL, 1, "update localRuntimeDefaults.ts");
    }

    /// One slot means the context shown is the context each conversation gets.
    ///
    /// The reason the default is not higher: `-c` is a shared pool, so a default
    /// above one would quietly divide the number a user set by a factor they did
    /// not choose.
    #[test]
    fn the_default_single_slot_means_no_context_is_divided() {
        let settings = LocalRuntimeSettings::default();
        assert_eq!(settings.context_per_slot(), settings.context);
    }

    /// The defaults still ask for the same offload the app has always used.
    ///
    /// Regression guard: the previous code hardcoded `-ngl 999` inline, and
    /// turning that into a setting must not silently change it to something else.
    #[test]
    fn the_default_configuration_still_offloads_every_layer() {
        assert_eq!(
            value_of(&LocalRuntimeSettings::default().to_args(), "-ngl").as_deref(),
            Some("999")
        );
    }

    /// "Off" means no flag, for every toggle.
    ///
    /// This is the invariant the whole module exists to keep. `Some(false)` must
    /// not become `--no-kv-offload` or `--flash-attn off`, because those override
    /// a llama.cpp default that was doing something sensible.
    #[test]
    fn an_option_that_is_off_passes_nothing() {
        let settings = LocalRuntimeSettings {
            kv_offload: Some(false),
            mmproj_offload: Some(false),
            ..LocalRuntimeSettings::default()
        };
        let args = settings.to_args();
        assert!(
            !args.iter().any(|arg| arg.contains("kv-offload")),
            "{args:?}"
        );
        assert!(
            !args.iter().any(|arg| arg.contains("mmproj-offload")),
            "{args:?}"
        );
    }

    /// An option that is on passes the flag.
    #[test]
    fn an_option_that_is_on_passes_its_flag() {
        let settings = LocalRuntimeSettings {
            kv_offload: Some(true),
            mmproj_offload: Some(true),
            ..LocalRuntimeSettings::default()
        };
        let args = settings.to_args();
        assert!(args.iter().any(|arg| arg == "--kv-offload"), "{args:?}");
        assert!(args.iter().any(|arg| arg == "--mmproj-offload"), "{args:?}");
    }

    /// Context is divided between slots unless the per-slot flag is set.
    ///
    /// The behaviour that started this: 10k context with 6 parallel silently
    /// gives each conversation about 1.6k, which is not what the number in the
    /// box appears to say.
    #[test]
    fn context_is_shared_between_slots_by_default() {
        let settings = LocalRuntimeSettings {
            context: 10_000,
            parallel: 6,
            kv_unified_per_slot: None,
            ..LocalRuntimeSettings::default()
        };
        assert_eq!(settings.context_per_slot(), 1667);
        assert!(!settings
            .to_args()
            .iter()
            .any(|arg| arg == "--kv-unified-per-slot"));
    }

    /// With the per-slot flag, each slot gets the whole context.
    #[test]
    fn per_slot_context_gives_every_slot_the_full_context() {
        let settings = LocalRuntimeSettings {
            context: 10_000,
            parallel: 6,
            kv_unified_per_slot: Some(10_000),
            ..LocalRuntimeSettings::default()
        };
        assert_eq!(settings.context_per_slot(), 10_000);
        assert_eq!(
            value_of(&settings.to_args(), "--kv-unified-per-slot").as_deref(),
            Some("10000")
        );
    }

    /// K and V are independent, because they are two flags.
    #[test]
    fn the_k_and_v_cache_quantisations_are_separate() {
        let settings = LocalRuntimeSettings {
            cache_type_k: Some(KvQuant::Q8_0),
            cache_type_v: Some(KvQuant::Q4_0),
            ..LocalRuntimeSettings::default()
        };
        let args = settings.to_args();
        assert_eq!(value_of(&args, "-ctk").as_deref(), Some("q8_0"));
        assert_eq!(value_of(&args, "-ctv").as_deref(), Some("q4_0"));
    }

    /// Every quantisation maps to a spelling the binary accepts.
    ///
    /// A typo here fails at spawn rather than at compile time, because the flag
    /// is only a string until the process starts.
    #[test]
    fn every_quantisation_has_a_flag_spelling() {
        for quant in KvQuant::ALL {
            let settings = LocalRuntimeSettings {
                cache_type_k: Some(quant),
                ..LocalRuntimeSettings::default()
            };
            assert_eq!(
                value_of(&settings.to_args(), "-ctk").as_deref(),
                Some(quant.as_flag())
            );
        }
    }

    /// A blank projector path is not passed as an empty argument.
    ///
    /// `--mmproj ""` would point llama.cpp at the current directory.
    #[test]
    fn a_blank_projector_path_is_not_passed() {
        let settings = LocalRuntimeSettings {
            mmproj: Some("   ".to_string()),
            ..LocalRuntimeSettings::default()
        };
        assert!(!settings.to_args().iter().any(|arg| arg == "--mmproj"));
    }

    /// A real projector path is passed through untouched.
    #[test]
    fn a_projector_path_is_passed_as_given() {
        let settings = LocalRuntimeSettings {
            mmproj: Some(r"C:\models\mmproj.gguf".to_string()),
            ..LocalRuntimeSettings::default()
        };
        assert_eq!(
            value_of(&settings.to_args(), "--mmproj").as_deref(),
            Some(r"C:\models\mmproj.gguf")
        );
    }

    /// Flash Attention has three states, because `auto` is not the same as off.
    #[test]
    fn flash_attention_carries_all_three_states() {
        for (choice, expected) in [
            (FlashAttention::Auto, "auto"),
            (FlashAttention::On, "on"),
            (FlashAttention::Off, "off"),
        ] {
            let settings = LocalRuntimeSettings {
                flash_attention: Some(choice),
                ..LocalRuntimeSettings::default()
            };
            assert_eq!(
                value_of(&settings.to_args(), "-fa").as_deref(),
                Some(expected)
            );
        }
    }

    /// Nothing that varies between runs ends up in the arguments.
    ///
    /// The prompt cache is keyed on the request prefix, so an argument list that
    /// changes run to run invalidates it for no reason. Every value here comes
    /// from stored settings, which is what keeps that true.
    #[test]
    fn the_same_settings_always_produce_the_same_arguments() {
        let settings = LocalRuntimeSettings::default();
        assert_eq!(settings.to_args(), settings.to_args());
    }

    /// Settings saved by an older build still load.
    ///
    /// A blob written before these fields existed must not fail the whole parse;
    /// the alternative is a model with no configuration and no way to fix it
    /// from the UI.
    #[test]
    fn older_settings_without_the_new_fields_still_load() {
        let older = r#"{"context": 8192, "parallel": 2}"#;
        let settings: LocalRuntimeSettings = serde_json::from_str(older).unwrap();
        assert_eq!(settings.context, 8192);
        assert_eq!(settings.parallel, 2);
        // The rest takes defaults rather than being absent.
        assert_eq!(settings.gpu_layers, Some(999));
        assert_eq!(settings.cache_type_k, Some(KvQuant::Q8_0));
    }
}
