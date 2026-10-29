/** The shortcuts the operating system owns. Mirrors `keybinds::Keybinds`. */
export type Keybinds = {
  /**
   * An accelerator (`Ctrl+Shift+Space`), or null for none.
   *
   * The string is produced by the settings page's recorder and validated by the
   * backend when it is registered, so nothing else parses it.
   */
  pushToTalk: string | null;
};
