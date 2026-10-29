# Chat translator with a local LLM — design

Status: approved in conversation on 2026-10-29, spec under review.
Scope: experiment on the `ahmadsayed/Aurora-Viewer` fork only (branch
`feat/chat-translator`); not proposed upstream.

## Goal

Let the user hold a private conversation (IM) with someone who writes
another language, and who may use any viewer. Translation runs both ways,
entirely on the user's machine, with a model dedicated to translation. Setup
must be one command or one click.

Success: the user types English in an IM set to Spanish; the other person
receives only Spanish; their Spanish replies show in English, original one
click away; the frame rate does not move while translating.

## Decisions

| Topic | Decision |
|---|---|
| Directions | Both: incoming and outgoing |
| Where | Private IMs only (no group chat, no nearby chat for now) |
| Language setting | Per conversation; "my language" is global, defaults to the UI language |
| Setup | `/tr <lang>` in the IM, or the header pill; auto suggestion when another language is detected |
| Outgoing | Sent automatically once translated; never sent untranslated without the user's choice |
| Runtime | llama.cpp embedded in the viewer (`llama-cpp-2`), Vulkan backend, CPU fallback |
| Model | Downloaded on first use after a consent modal, checksum-pinned, from a small built-in catalogue |
| Build | Cargo feature `translator`, off by default |

## Components

### Crate `crates/aurora-translate` (no UI, no network protocol)

- `Translator` — owns a worker thread with the llama.cpp context. Jobs
  `{id, text, from, to, direction}` arrive on a channel; results
  `Result<String, TranslateError>` go back on another. One job at a time,
  outgoing jobs ahead of incoming ones. The model loads on the first job and
  unloads after 10 minutes without a job.
- `prompt.rs` — the prompt template of the chosen model (the incoming text is
  wrapped as data, never as instructions) and output cleanup: strip quotes
  and "Translation:"-style prefixes; reject an empty output, an output more
  than 4× the input length, or one that starts with assistant chatter.
- `model.rs` — catalogue entries `{id, label, url, bytes, sha256, prompt
  kind}`. Default: a translation-dedicated model of about 4B parameters,
  Q4 GGUF (TranslateGemma 4B; exact repository, file and checksum are
  confirmed at implementation time). One larger option (for example
  Hunyuan-MT-7B) for quality. Resumable download into the viewer's data
  folder (`models/`), SHA-256 verified, deleted on mismatch, disk space
  checked first, cancellable.
- `detect.rs` — language detection with `whatlang` (pure Rust, MIT): language
  and confidence.
- `lang.rs` — the supported languages (about 30): ISO 639-1 code, English
  name, native name; parses `es`, `spanish`, `español` (case and accent
  insensitive).
- `FakeTranslator` — same interface, deterministic output (tag + text), for
  tests and the demo.

The `Translator` interface is a trait so that the fake and the llama.cpp
backend swap freely.

### In `aurora-viewer`, behind `#[cfg(feature = "translator")]`

- `translate/mod.rs` — app-side glue:
  - per-conversation state `HashMap<SessionId, Lang>`, plus "suggestion
    dismissed" flags, persisted with the chat settings;
  - jobs in flight, mapped to transcript lines;
  - `poll()` once per frame with `try_recv` only: never blocks.
- Outgoing hook: in the `a.ims` loop of `app.rs`, after `colon_emote` and
  `expand_shortcodes`, a session with a partner language submits a job
  instead of `NetCommand::SendIm`. Group sessions are untouched.
- Incoming hook: a received IM in a translated session submits a job; the
  line is displayed at once in the original.
- Commands: `/tr <lang>`, `/tr off`, `/tr` (status) handled in the IM input
  before sending, next to the existing `cmdline` commands.
- UI (theme rules of AGENTS.md section 7, labels through `tr!` in English,
  French and Japanese):
  - header pill in `ui/chat/header.rs`: "EN ⇄ ES", menu of languages, Off,
    "Preview before sending" toggle (off by default);
  - suggestion card in the conversation;
  - download modal (`widgets::Modal`, `widgets::progress`);
  - Preferences > Chat > "Translation": my language, model, delete model.

## Data flow

### Outgoing

1. Enter → existing text processing → `translate::submit_outgoing`.
2. The English line shows at once, dimmed, with a "translating…" spinner.
3. Success → `SendIm` with the translation; the line shows the translation,
   the original below in small muted text (what the other person got is what
   the user sees).
4. Failure or 10 s timeout → nothing sent; the line offers **Retry** and
   **Send original**.
5. Translation longer than 1023 bytes (IM limit) → **Send original** or
   **Edit**.
6. `/me`: only the text after `/me` is translated. Typing indicators are
   unchanged.
7. Messages of one conversation are sent in typed order: a later message
   waits for an earlier one still translating.

### Incoming

1. The line shows the original at once.
2. Success → the line shows the translation, the original below in small
   muted text; a click swaps them.
3. Failure → original stays, with a "not translated" marker and retry.
4. The chat log on disk keeps both texts.

### Suggestion

In an IM with translation off: two consecutive incoming messages of at least
15 characters detected, with high confidence, in the same language that is not
"my language" → card "They seem to write Spanish. Translate this chat?
[EN ⇄ ES] [Not now]". "Not now" silences it for that conversation.

### First use

`/tr es` (or the card, or the pill) with no model on disk → download modal
with size and model name → download with progress → translation starts. If
the user declines, the conversation stays untranslated.

## Errors

- Model load failure on Vulkan (VRAM…) → one retry on CPU; the backend used
  is logged and shown in the pill tooltip ("Running on CPU (slower)").
- No `unwrap`/`expect` on the model file, downloads or network text: errors
  become `TranslateError`, are logged and shown on the line.
- Worker thread death → the app sees a closed channel, marks the translator
  offline, pending lines get Retry / Send original.
- Privacy: nothing leaves the machine except the model download; logs hold
  lengths and timings, never message text.

## Performance

- No inference on the main thread; the frame loop only polls.
- Small context (512 tokens), one model loaded at a time, idle unload.
- Measured with `AURORA_PROFILE=1`: frame time with and without translation
  in progress; tokens/s logged per job.

## Testing

- Unit tests: language parsing, output cleanup, suggestion thresholds,
  outgoing state machine (pending → sent / failed / too long, ordering),
  catalogue and checksum verification on fake files.
- `cargo test --workspace --features aurora-viewer/translator`
  (excluding `aurora-render` locally, as usual on this Linux setup).
- Demo: `AURORA_DEMO_TRANSLATE=1` — a demo avatar writes Spanish in an IM:
  suggestion card, pill, translated lines both ways, with `FakeTranslator`;
  `AURORA_TRANSLATE_REAL=1` uses the real model.
- Human: real-model quality and speed in the demo; a grid IM with a person on
  another viewer.

## Docs

`docs/TESTING.md` (new variables), `ARCHITECTURE.md` (new crate and module),
`TASKS.md` line on the fork branch.

## Out of scope

Group chat and nearby chat translation, custom GGUF file, external servers
(Ollama), translation of notices and object chat, upstream PR.
