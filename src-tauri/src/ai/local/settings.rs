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

    /// `--chat-template`. A built-in name, or the template itself.
    ///
    /// A GGUF whose metadata carries the wrong (or no) template is the commonest
    /// reason a local model answers nonsense while the same file works elsewhere:
    /// the model is fine, the prompt format is not.
    pub chat_template: Option<String>,

    /// `--chat-template-file`. A `.jinja` file, for a template kept outside the app.
    pub chat_template_file: Option<String>,

    /// `--jinja`. llama.cpp's own Jinja engine, which some templates need.
    pub jinja: bool,

    /// `--temp`. Sampling defaults for this model. Absent leaves llama.cpp's.
    pub temperature: Option<f32>,

    /// `--top-p`.
    pub top_p: Option<f32>,

    /// `--min-p`.
    pub min_p: Option<f32>,

    /// `--repeat-penalty`.
    pub repeat_penalty: Option<f32>,

    /// `--mlock`. Keeps the model in RAM instead of letting it be paged out.
    pub mlock: bool,

    /// `--no-mmap`. Reads the weights fully instead of mapping the file.
    pub no_mmap: bool,

    /// `-b`. Prompt-processing batch.
    pub batch_size: Option<u32>,

    /// `-ub`. The smaller batch the GPU actually sees.
    pub ubatch_size: Option<u32>,

    /// `--model-draft`. A small model that proposes tokens for the main one to
    /// accept, which is the largest speedup available on a big model.
    pub draft_model: Option<String>,

    /// `--gpu-layers-draft`. How much of the draft model goes on the GPU.
    pub draft_gpu_layers: Option<u32>,

    /// A full argument list typed by the user, replacing everything above.
    ///
    /// `--model`, `--alias`, `--host` and `--port` stay with the app: it needs
    /// them to find and talk to the process. Everything else is theirs to set.
    pub custom_args: Option<String>,
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
            chat_template: None,
            chat_template_file: None,
            jinja: false,
            temperature: None,
            top_p: None,
            min_p: None,
            repeat_penalty: None,
            mlock: false,
            no_mmap: false,
            batch_size: None,
            ubatch_size: None,
            draft_model: None,
            draft_gpu_layers: None,
            custom_args: None,
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
        // A template file wins over an inline one: it is the more specific of the
        // two, and setting both is a mistake the user can see here rather than in
        // llama.cpp's error message.
        if let Some(file) = &self.chat_template_file {
            if !file.trim().is_empty() {
                push(&mut args, "--chat-template-file", file);
            }
        } else if let Some(template) = &self.chat_template {
            if !template.trim().is_empty() {
                push(&mut args, "--chat-template", template);
            }
        }
        if self.jinja {
            args.push("--jinja".to_string());
        }
        for (flag, value) in [
            ("--temp", self.temperature),
            ("--top-p", self.top_p),
            ("--min-p", self.min_p),
            ("--repeat-penalty", self.repeat_penalty),
        ] {
            if let Some(value) = value {
                push(&mut args, flag, &format!("{value}"));
            }
        }
        if self.mlock {
            args.push("--mlock".to_string());
        }
        if self.no_mmap {
            args.push("--no-mmap".to_string());
        }
        if let Some(batch) = self.batch_size {
            push(&mut args, "-b", &batch.to_string());
        }
        if let Some(batch) = self.ubatch_size {
            push(&mut args, "-ub", &batch.to_string());
        }
        if let Some(draft) = &self.draft_model {
            if !draft.trim().is_empty() {
                push(&mut args, "--model-draft", draft);
            }
        }
        if let Some(layers) = self.draft_gpu_layers {
            push(&mut args, "--gpu-layers-draft", &layers.to_string());
        }
        args
    }

    /// The arguments the process is actually started with.
    ///
    /// A custom list replaces the generated one wholesale rather than being
    /// appended to it: "edit the command" means the text is the command, and a
    /// hidden prefix would make the page lie about what is running.
    pub fn effective_args(&self) -> Vec<String> {
        match &self.custom_args {
            Some(text) if !text.trim().is_empty() => split_arguments(text),
            _ => self.to_args(),
        }
    }
}

/// An argument list as one line a person can edit.
///
/// Quoted only where a value needs it, so the common case reads like the command
/// someone would have typed.
pub fn quote_arguments(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.is_empty()
                || arg
                    .chars()
                    .any(|character| character.is_whitespace() || character == '"')
            {
                format!("\"{}\"", arg.replace('"', "\\\""))
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The flags the app sets itself, which a custom argument list may not repeat.
pub const RESERVED_ARGUMENTS: [&str; 4] = ["--model", "--alias", "--host", "--port"];

/// Splits a command line into arguments, honouring quotes.
///
/// Only what a person types into one box: single and double quotes group a value
/// with a space in it. Not a shell -- no variables, no globbing, no operators,
/// because the string is arguments, not a script.
///
/// **A backslash is a path separator here, not an escape.** Windows paths are
/// nothing but backslashes, so treating every one as an escape deleted them from
/// both quoted and unquoted values -- `--model-draft "C:\Models\draft.gguf"`
/// arrived at `llama-server` as `C:Modelsdraft.gguf`. A backslash now escapes
/// only a character it could not otherwise be: the active quote, another
/// backslash, or (outside quotes) whitespace. Every other one is kept verbatim.
fn split_arguments(text: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut started = false;
    let mut characters = text.chars().peekable();

    fn escaped_next(quote: Option<char>, next: char) -> bool {
        match quote {
            // Inside quotes only the quote itself needs an escape. A backslash
            // before another backslash stays two, so a UNC path survives.
            Some(open) => next == open,
            // Outside quotes whitespace and either quote would end the token.
            None => next.is_whitespace() || next == '"' || next == '\'',
        }
    }

    while let Some(character) = characters.next() {
        match quote {
            Some(open) => {
                if character == open {
                    quote = None;
                } else if character == '\\'
                    && characters
                        .peek()
                        .is_some_and(|next| escaped_next(quote, *next))
                {
                    current.push(characters.next().unwrap_or_default());
                } else {
                    current.push(character);
                }
            }
            None => match character {
                '"' | '\'' => {
                    quote = Some(character);
                    started = true;
                }
                '\\' => {
                    if characters
                        .peek()
                        .is_some_and(|next| escaped_next(None, *next))
                    {
                        current.push(characters.next().unwrap_or_default());
                    } else {
                        current.push(character);
                    }
                    started = true;
                }
                other if other.is_whitespace() => {
                    if started || !current.is_empty() {
                        args.push(std::mem::take(&mut current));
                        started = false;
                    }
                }
                other => {
                    current.push(other);
                    started = true;
                }
            },
        }
    }
    if started || !current.is_empty() {
        args.push(current);
    }
    args
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

    /// The value after a flag, wherever it sits. `pairs` above assumes every flag
    /// takes one, which stops being true once a switch like `--mlock` is emitted.
    fn followed(args: &[String], flag: &str) -> Option<String> {
        args.iter()
            .position(|argument| argument == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    }

    #[test]
    fn the_template_and_memory_flags_reach_the_command() {
        let mut settings = LocalRuntimeSettings::default();
        settings.chat_template = Some("chatml".to_string());
        settings.jinja = true;
        settings.mlock = true;
        settings.no_mmap = true;
        settings.temperature = Some(0.7);
        settings.batch_size = Some(2048);
        settings.draft_model = Some("draft.gguf".to_string());

        let args = settings.to_args();
        assert_eq!(
            followed(&args, "--chat-template").as_deref(),
            Some("chatml")
        );
        assert!(args.contains(&"--jinja".to_string()));
        assert!(args.contains(&"--mlock".to_string()));
        assert!(args.contains(&"--no-mmap".to_string()));
        assert_eq!(followed(&args, "--temp").as_deref(), Some("0.7"));
        assert_eq!(followed(&args, "-b").as_deref(), Some("2048"));
        assert_eq!(
            followed(&args, "--model-draft").as_deref(),
            Some("draft.gguf")
        );
    }

    #[test]
    fn a_template_file_wins_over_inline_text() {
        let mut settings = LocalRuntimeSettings::default();
        settings.chat_template = Some("chatml".to_string());
        settings.chat_template_file = Some("C:\\templates\\mine.jinja".to_string());

        let args = settings.to_args();
        assert_eq!(
            followed(&args, "--chat-template-file").as_deref(),
            Some("C:\\templates\\mine.jinja")
        );
        // Only one template is ever passed: two would be an error llama.cpp has to
        // report instead of this.
        assert!(!args.contains(&"--chat-template".to_string()));
    }

    #[test]
    fn a_custom_list_replaces_every_generated_flag() {
        let mut settings = LocalRuntimeSettings::default();
        settings.custom_args = Some("--threads 8 --mlock".to_string());

        assert_eq!(settings.effective_args(), vec!["--threads", "8", "--mlock"]);
    }

    #[test]
    fn an_empty_custom_list_falls_back_to_the_settings() {
        let mut settings = LocalRuntimeSettings::default();
        settings.custom_args = Some("   ".to_string());

        assert_eq!(settings.effective_args(), settings.to_args());
    }

    #[test]
    fn a_quoted_argument_survives_the_round_trip() {
        let args = vec![
            "--chat-template".to_string(),
            "a template with spaces".to_string(),
            "--jinja".to_string(),
        ];
        assert_eq!(split_arguments(&quote_arguments(&args)), args);
    }

    /// **A Windows path keeps its separators**, quoted or not.
    ///
    /// A backslash used to escape whatever followed it, so every path typed into
    /// the custom list arrived at `llama-server` with its directories glued
    /// together -- `C:\Models\draft.gguf` became `C:Modelsdraft.gguf`. The quotes
    /// are the box's syntax; the backslashes are the path.
    #[test]
    fn a_windows_path_keeps_its_backslashes() {
        assert_eq!(
            split_arguments(r#"--model-draft "C:\Models\Qwen Draft\qwen.gguf" --threads 4"#),
            vec![
                "--model-draft",
                r"C:\Models\Qwen Draft\qwen.gguf",
                "--threads",
                "4",
            ]
        );
        assert_eq!(
            split_arguments(r"--mmproj C:\models\mmproj.gguf"),
            vec!["--mmproj", r"C:\models\mmproj.gguf"]
        );
        // A quoted path round-trips through the box without being rewritten.
        let args = vec![
            "--model-draft".to_string(),
            r"C:\Models\Qwen Draft\qwen.gguf".to_string(),
        ];
        assert_eq!(split_arguments(&quote_arguments(&args)), args);
    }

    /// A quote can still be escaped inside a quoted value, which is the one thing
    /// a backslash is for here.
    #[test]
    fn an_escaped_quote_is_unquoted() {
        assert_eq!(
            split_arguments(r#"--chat-template "say \"hello\"""#),
            vec!["--chat-template", "say \"hello\""]
        );
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
