# Screenshot guide

Save every file in this folder using the exact name below. `README.md` uses five of them:
`chat-agent`, `chat-empty`, `statistics`, `settings-models` and `settings-voice`. The rest are
kept as optional extras.

## Capture rules

- **Window only.** Crop to the app, no desktop, taskbar or shadows. Maximise the window.
- **1600 × 1000 or larger**, then export at 2× (PNG). Phone screenshots are fine for the
  optional Telegram shot, but crop the status bar.
- **One theme throughout.** Dark is the better hero; if you shoot light, shoot every shot
  light.
- **Real data, no placeholders.** Several conversations, real file names, real token counts.
  Empty charts and "No jobs yet" undersell the app.
- **Censor the private bits:** API keys, bot token, Telegram user ids, personal folder paths,
  email addresses, anything under a client's name. Use a real-looking fake path like
  `C:\Projects\atlas` if needed.
- **No cursor, no hover tooltips, no settings search open** unless the shot is about them.
- PNG, not JPEG — the UI is mostly text.

---

## Required

### 1. `chat-agent.png` — the hero

**Page:** Chat, an Agent conversation.

Put on screen:

- A real task in progress or just finished: "add a settings toggle and update the docs", "review
  this module", "build the landing page".
- The activity panel expanded under the reply: **"Worked for 42s · 6 steps"**, with at least one
  read, one `edit_file` showing **+120 / −8**, and one terminal call.
- A Markdown answer with a heading, a bullet list and a short code block.
- The **right panel open on Files**, showing a real project tree and a file preview (Markdown or
  image is best).
- Sidebar with 6–10 believable conversation titles, one pinned, grouped under Today / Yesterday.
- Header showing a workspace folder and, if you have one loaded, the resident-model chip
  (`Qwen 3.5 4B · …`).
- Composer in **Agent** mode with **Access: Auto writes**, a model name, Effort and the context
  ring visible.

### 2. `chat-empty.png` — first impression

**Page:** Chat → new conversation.

- Set a name first: **Settings → General → Your name**, so the greeting reads
  "Good evening, Alex".
- No draft text. The composer shows a real model, Agent access level, effort and a partly
  filled context ring.
- Sidebar visible with history, so the window does not look empty.

### 3. `statistics.png` — the numbers page

**Page:** Statistics.

- Timeframe **Month** (or **3 months** if you have enough history); the header should read
  `Usage since … · times in UTC`.
- The four figure cards need non-zero values and, if possible, comparison pills with a rise or
  fall rather than "none in prior period".
- **Tokens over time** chart with visible variation, not a flat line.
- **Tools** panel with several ranked tool calls.
- **Models** table populated, and a few rows in **Conversations**.
- Use real usage: run several agent replies and a job before shooting.

### 4. `settings-models.png` — supply

**Page:** Settings → **Providers** (the list), or **Model → Main model** if you prefer a
settings page.

- Providers: two or three real rows with names, model counts, enabled toggles and the provider
  icons; masked keys. This page shows both the remote and the local side of the app.
- Alternative: **Providers → Local** with a GGUF model row, its engine group and the context
  slider visible. Good if you want the local-first story instead.

### 5. `settings-voice.png` — speech

**Page:** Settings → **Voice → Text to speech**.

- An installed voice row — Piper or audio.cpp — with its size and, for Piper, the pace control and **Test** button.
- If audio.cpp is installed, keep its voice set visible too — it shows a second engine.
- Do not open a download page; this is a finished setup, not an installation in progress.

---

## Optional, if they turn out well

| File | Page | Why |
|---|---|---|
| `jobs.png` | Jobs | Schedules, lanes and machine checks; one row expanded on **Runs** |
| `settings-taste.png` | Settings → **Taste** | The learned profile with editable facts and a recent run |
| `right-panel-subagents.png` | Chat + right panel → **Sub agents** | A delegated run with its own transcript and steps |
| `settings-local.png` | Settings → **Providers → Local** | Engine groups, GGUF rows, context slider |
| `telegram.png` | Settings → **Communications → Telegram** (connected) | The agent outside the app; mask the bot token and any user ids |
| `overview.png` | Sidebar → right-click a chat → **Overview** | A read-only session record: models, tools, tokens |

Keep the final README to five or six images. More than that and nobody scrolls.
