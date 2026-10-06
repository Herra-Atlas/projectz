/**
 * Frontend copies of the default runtime settings.
 *
 * ## These duplicate Rust, deliberately
 *
 * `DEFAULT_CONTEXT` and `DEFAULT_PARALLEL` are defined in
 * `src-tauri/src/ai/local/settings.rs` and are authoritative. They are restated
 * here for exactly one use: the fallback the settings page shows when
 * `local_runtime_settings` fails, so a backend error leaves the controls showing
 * usable numbers rather than a blank page.
 *
 * They are not read on the normal path -- the backend returns its own defaults
 * for any model with no stored settings, and that is what the page uses. So a
 * disagreement here changes only the appearance of a failure, never the value a
 * model is actually loaded with. Keeping them local is still worth the risk
 * because importing from the Tauri crate into the frontend is not possible, and
 * fetching them over IPC on every page load to avoid duplicating two integers
 * would be worse than the duplication.
 *
 * **If you change one, change the other.** The pair is asserted against the Rust
 * values in `settings.rs`'s `defaults_match_the_documented_values` test.
 */

export const DEFAULT_CONTEXT = 30_000;
export const DEFAULT_PARALLEL = 1;