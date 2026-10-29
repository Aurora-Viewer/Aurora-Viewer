# Chat Translator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Two-way translation of private IMs with a local llama.cpp model, set per conversation with `/tr <lang>`, a header pill or an auto suggestion.

**Architecture:** A new UI-free crate `aurora-translate` holds languages, detection, prompts, the model catalogue/download and a worker thread behind a `Backend` trait (a fake backend always, the llama.cpp backend behind its `llama` feature). The viewer gets a `translate` module (per-conversation state, line table, outgoing state machine, per-frame polling) wired into the IM send loop of `app.rs` and `World::on_im`, plus the chat UI (line extras, pill, card, modal, preferences).

**Tech Stack:** Rust 2024, `llama-cpp-2` (feature `vulkan`), `whatlang`, `reqwest` (blocking, rustls), `sha2`, `crossbeam-channel`, egui.

**Spec:** `docs/superpowers/specs/2026-10-29-chat-translator-design.md`

## Global Constraints

- Fork-only experiment: branch `feat/chat-translator` in `../work/chat-translator`, based on `upstream/main`; never opened upstream.
- Linux, no pwsh: tests are `cargo test --workspace --exclude aurora-render <filter>`; formatting `cargo fmt --check`; clippy `cargo clippy --workspace --exclude aurora-render --all-targets -- -D warnings` (pre-existing Windows-only dead-code warnings are not ours: compare with `upstream/main`).
- **Deviation from the spec, decided here:** the viewer always compiles the `translate` module and depends on `aurora-translate`; only the llama.cpp backend is optional. Viewer feature `translator` = `aurora-translate/llama`. Without it, `/tr` answers "Translation is not built into this viewer". Reason: no `cfg` scattered through `app.rs`, `world` and the UI.
- Private IMs only: group sessions (`world.is_group_session`) and nearby chat are never translated.
- Never sent untranslated without the user's choice. Outgoing timeout 10 s. IM limit 1023 bytes (UTF-8).
- Suggestion: 2 consecutive incoming messages ≥ 15 characters, same detected language, confidence ≥ 0.9 (`whatlang::Info::confidence`), not "my language".
- Worker: one job at a time, outgoing before incoming, model unload after 10 min idle, context 512 tokens.
- Output rejected when empty, longer than 4× the input (chars), or starting (case-insensitive) with "sure", "here is", "here's", "certainly".
- No `unwrap`/`expect` on model files, downloads or message text. Logs hold lengths and timings, never message text.
- UI follows AGENTS.md section 7 (Palette, `widgets::*`, Tabler icons); every label through `aurora_i18n::tr!` with keys in `en.ftl`, `fr.ftl`, `ja.ftl`. Identifiers in English.
- Commits in the human's name, no AI mention, imperative English messages.

## Review Focus

1. **Conversation closed or logout while a job is in flight** → the result is dropped; no `SendIm` after logout (Task 6 test `results_for_a_closed_session_are_dropped`, Task 7 clears on logout).
2. **Offline message with the "(Saved <date>)" prefix** → only the message body is translated; the prefix is kept (Task 7 test `saved_prefix_is_kept_out_of_the_job`).
3. **Incoming text that is only a URL, SLURL, emoji or number** → not sent to the model, shown as is (Task 1 test `nothing_to_translate`).
4. **Partial or corrupt model file on disk** (crash mid-download) → never loaded; resumed or downloaded again (Task 4 test `partial_file_is_not_ready`).
5. **Partner answers in my own language while translation is on** → not translated (Task 6 test `incoming_in_my_language_is_not_translated`).

---

### Task 1: Crate, languages and detection

**Files:**
- Create: `crates/aurora-translate/Cargo.toml`, `src/lib.rs`, `src/lang.rs`, `src/detect.rs`
- Modify: `ARCHITECTURE.md` ("The crates" table and file tree)

**Interfaces:**
- Produces:
  - `pub struct Lang { pub code: &'static str, pub english: &'static str, pub native: &'static str }`, `pub static LANGS: &[Lang]` (30 entries: en es fr de it pt nl pl ru uk tr ar he fa hi bn id vi th zh ja ko sv da no fi cs el ro hu).
  - `pub fn parse_lang(input: &str) -> Option<&'static Lang>`: matches the code, English name or native name, case and accent insensitive (`"ES"`, `"spanish"`, `"Español"`, `"espanol"` → es).
  - `pub fn lang(code: &str) -> Option<&'static Lang>`.
  - `pub struct Detected { pub lang: &'static Lang, pub confidence: f64 }`, `pub fn detect(text: &str) -> Option<Detected>` (maps `whatlang::Lang` to `LANGS`; `None` when unmapped).
  - `pub fn nothing_to_translate(text: &str) -> bool`: true when the text, after removing URLs (`http(s)://`, `secondlife://`), emoji/symbols, digits and punctuation, has fewer than 2 letters.

- [ ] **Step 1:** Create the crate (`edition.workspace`, `license.workspace`, `lints.workspace = true`), deps `whatlang`, `log`, `thiserror`; `[features] llama = []` (filled in Task 5).
- [ ] **Step 2: Failing tests** in `lang.rs` / `detect.rs`:
  - `parse_lang_accepts_codes_and_names`: the four inputs above → `"es"`; `"japanese"`, `"日本語"` → `"ja"`; `"klingon"` → `None`.
  - `detect_spanish`: `detect("¿Dónde estás? Te estoy buscando en la tienda.")` → `es`, confidence ≥ 0.9.
  - `nothing_to_translate`: true for `"https://example.com/x"`, `"secondlife:///app/agent/…/about"`, `"😀👍"`, `"42!"`, `"ok"`; false for `"hola amigo"`.
- [ ] **Step 3:** Run `cargo test --workspace --exclude aurora-render lang` → FAIL (not defined).
- [ ] **Step 4:** Implement. Accent folding: NFD is not needed; a small table for á é í ó ú ü ñ ç à è ì ò ù â ê î ô û ä ö å ø æ → base letter is enough for the 30 native names.
- [ ] **Step 5:** Tests pass; add the crate to `ARCHITECTURE.md`; commit `Add the aurora-translate crate with languages and detection`.

### Task 2: Prompt and output cleanup

**Files:** Create `crates/aurora-translate/src/prompt.rs`

**Interfaces:**
- Consumes: `Lang` (Task 1).
- Produces:
  - `pub enum PromptKind { TranslateGemma, HunyuanMt }` (serde, `Copy`).
  - `pub fn user_message(kind: PromptKind, text: &str, from: &Lang, to: &Lang) -> String` — the user turn passed to the model's own chat template (Task 5 applies the template).
  - `pub fn clean_output(raw: &str, input: &str) -> Result<String, Rejected>`, `pub enum Rejected { Empty, TooLong, Chatter }`.

- [ ] **Step 1:** Open the model cards of TranslateGemma 4B and Hunyuan-MT-7B (Hugging Face) and copy their recommended translation prompt into `user_message`, with `{from}`/`{to}` as English names and codes and the text last, after a blank line. If a card gives none, use: `Translate the following text from {from.english} ({from.code}) to {to.english} ({to.code}). Output only the translation, nothing else.\n\n{text}`. Put the card URL in a comment.
- [ ] **Step 2: Failing tests:**
  - `user_message_ends_with_the_text`: text is the suffix; both language names appear.
  - `clean_strips_quotes_and_prefixes`: `"\"Hola\""` → `Hola`; `"Translation: Hola"` → `Hola`; `"Spanish: Hola\n"` → `Hola`.
  - `clean_rejects`: `""`/`"   "` → `Empty`; 41-char output for 10-char input → `TooLong`; `"Sure, here is the translation: Hola"` → `Chatter`.
- [ ] **Step 3:** Run, FAIL. **Step 4:** Implement (trim, strip one pair of matching `"`/`“”`/`«»`, strip a leading `<word>:` when the word is "translation" or a language English/native name). **Step 5:** PASS, commit `Add translation prompts and output cleanup`.

### Task 3: Worker thread and fake backend

**Files:** Create `crates/aurora-translate/src/worker.rs`, `src/fake.rs`

**Interfaces:**
- Consumes: `PromptKind`, `user_message`, `clean_output`, `Lang`.
- Produces:
  - `pub trait Backend: Send + 'static { fn load(&mut self) -> Result<BackendInfo, TranslateError>; fn generate(&mut self, user_message: &str, max_tokens: usize) -> Result<String, TranslateError>; fn unload(&mut self); }`
  - `pub struct BackendInfo { pub on_gpu: bool }`
  - `#[derive(thiserror::Error)] pub enum TranslateError { NotBuilt, ModelMissing, Load(String), Generate(String), Rejected(Rejected), Closed }`
  - `pub enum Direction { Outgoing, Incoming }`
  - `pub struct Job { pub id: u64, pub text: String, pub from: &'static Lang, pub to: &'static Lang, pub direction: Direction }`
  - `pub struct Done { pub id: u64, pub result: Result<String, TranslateError>, pub millis: u32 }`
  - `pub struct Translator` with `pub fn spawn(backend: Box<dyn Backend>, kind: PromptKind, idle_unload: Duration) -> Translator`, `pub fn submit(&self, job: Job)`, `pub fn try_recv(&self) -> Option<Done>`, `pub fn status(&self) -> Status` (`Status { loaded: bool, on_gpu: bool }`, shared through an `Arc<Mutex<_>>` updated by the worker).
  - `pub struct FakeBackend` — `generate` returns `"[{to}] {text}"` (the text and `to` code parsed back from the user message's last paragraph and the `(xx)` codes); `FakeBackend::failing()` always returns `Generate`; `FakeBackend::slow(Duration)`.

- [ ] **Step 1: Failing tests** (`worker.rs`, with `FakeBackend`):
  - `translates_a_job`: submit, poll with a 2 s deadline → `Ok("[es] hello")`.
  - `outgoing_jumps_the_queue`: with `slow(100ms)`, submit incoming 1, 2, then outgoing 3 while 1 runs → completion order 1, 3, 2.
  - `unloads_when_idle`: `idle_unload = 50ms` → `status().loaded` false after 200 ms.
  - `errors_come_back`: `failing()` → `Err(Generate(_))`.
  - `drop_stops_the_thread`: dropping `Translator` joins the thread within 1 s.
- [ ] **Step 2:** FAIL. **Step 3:** Implement: two `crossbeam_channel` queues (outgoing, incoming) + a stop flag; the worker `select!`s outgoing first (`try_recv` outgoing, then `recv_timeout` on both with the idle deadline); loads lazily; `max_tokens = 4 × input chars + 32`, capped at 400; `clean_output` applied. **Step 4:** PASS, commit `Add the translation worker and a fake backend`.

### Task 4: Model catalogue and download

**Files:** Create `crates/aurora-translate/src/model.rs`; Modify `Cargo.toml` (deps `reqwest` with `blocking` + `rustls`, `sha2`, `serde`)

**Interfaces:**
- Produces:
  - `pub struct ModelEntry { pub id: &'static str, pub label: &'static str, pub url: &'static str, pub bytes: u64, pub sha256: &'static str, pub prompt: PromptKind }`, `pub static CATALOGUE: &[ModelEntry]` — `translategemma-4b-q4` (default) and `hunyuan-mt-7b-q4`. Fill `url`, `bytes`, `sha256` from the Hugging Face GGUF repositories (Q4_K_M files); note the repo in a comment.
  - `pub fn entry(id: &str) -> Option<&'static ModelEntry>`, `pub const DEFAULT_MODEL: &str = "translategemma-4b-q4"`.
  - `pub fn model_path(dir: &Path, e: &ModelEntry) -> PathBuf` (`<dir>/<id>.gguf`; partial file `<id>.gguf.part`).
  - `pub fn is_ready(dir: &Path, e: &ModelEntry) -> bool` — final file exists with size `e.bytes` (checksum is verified once, before the rename).
  - `pub struct Download` with `pub fn start(dir: PathBuf, e: &'static ModelEntry) -> Download`, `pub fn progress(&self) -> DownloadState`, `pub fn cancel(&self)`; `pub enum DownloadState { Running { done: u64, total: u64 }, Verifying, Ready(PathBuf), Failed(DownloadError), Cancelled }`; `pub enum DownloadError { NoSpace { needed: u64 }, Http(String), Io(String), Checksum }`.
  - `fn verify(path: &Path, sha256: &str) -> Result<(), DownloadError>` (`pub(crate)` for tests).

- [ ] **Step 1: Failing tests** (temp dir, no network):
  - `partial_file_is_not_ready`: only `<id>.gguf.part` present → `false`; final file of wrong size → `false`.
  - `verify_detects_a_bad_checksum`: a 3-byte file against the SHA-256 of `"abc"` → Ok; against another hash → `Checksum`.
  - `catalogue_is_consistent`: ids unique, `DEFAULT_MODEL` present, every `sha256` 64 hex chars, `bytes > 0`.
- [ ] **Step 2:** FAIL. **Step 3:** Implement on a background thread: check free space with `fs4::available_space` (dep `fs4`, MIT/Apache; if the call fails, log and go on), resume with `Range: bytes=<part len>-`, stream to `.part`, verify, rename; on checksum failure delete `.part`. **Step 4:** PASS, commit `Add the translation model catalogue and download`.

### Task 5: llama.cpp backend

**Files:** Create `crates/aurora-translate/src/llama.rs` (`#[cfg(feature = "llama")]`); Modify `Cargo.toml` (`llama-cpp-2 = { version = "<latest>", optional = true, features = ["vulkan"] }`, `llama = ["dep:llama-cpp-2"]`), `src/lib.rs`

**Interfaces:**
- Consumes: `Backend`, `BackendInfo`, `TranslateError`.
- Produces: `pub struct LlamaBackend` with `pub fn new(model_path: PathBuf) -> LlamaBackend`; `pub fn backend_for(model_path: PathBuf) -> Box<dyn Backend>` in `lib.rs` returning `LlamaBackend` with the feature and a backend whose `load` returns `Err(NotBuilt)` without it.

- [ ] **Step 1:** Implement `load`: `LlamaBackend::init()` once (keep it in a `OnceLock`), load the model with all layers on GPU; on error, log and retry with `n_gpu_layers = 0` (`on_gpu: false`). Context `n_ctx = 512`. `generate`: apply the model's chat template (`model.chat_template` + `apply_chat_template`) to one user turn, tokenize, decode, greedy sampling until EOG or `max_tokens`. `unload` drops context and model. Check the exact calls against the `llama-cpp-2` docs of the version pinned.
- [ ] **Step 2: Test** `real_model_translates` marked `#[ignore]`, reading `AURORA_TRANSLATE_MODEL=<path to gguf>`: `"Hello, how are you?"` en→es contains `"hola"` (case-insensitive).
- [ ] **Step 3:** `cargo build -p aurora-translate --features llama` (Vulkan SDK headers: `vulkan-headers`, `glslc` on Fedora — note them in `docs/LINUX.md`); run the ignored test with a downloaded model; log tokens/s. Commit `Add the llama.cpp translation backend`.

### Task 6: Viewer state and outgoing state machine

**Files:** Create `crates/aurora-viewer/src/translate/mod.rs`, `translate/state.rs`, `translate/tests.rs`; Modify `crates/aurora-viewer/Cargo.toml` (dep `aurora-translate`, feature `translator = ["aurora-translate/llama"]`), `src/settings.rs` (field), `src/main.rs`/`lib.rs` (`mod translate;`)

**Interfaces:**
- Consumes: Task 1–4 types.
- Produces:
  - `settings.rs`: `#[serde(default)] pub translate: TranslateSettings` with `pub my_language: String` (empty = UI language), `pub model: String` (`DEFAULT_MODEL`), `pub preview: bool` (false), `pub sessions: HashMap<Uuid, String>` (partner lang code), `pub dismissed: HashSet<Uuid>`.
  - `pub struct LineKey { pub session: Uuid, pub time: SystemTime }` (`Hash`, `Eq`).
  - `pub enum LineState { Pending, Translated { original: String }, Failed { reason: FailReason }, TooLong { translation: String } }`, `pub enum FailReason { Error, Timeout }`.
  - `pub struct Translations` (pure logic, no thread), methods:
    - `pub fn outgoing(&mut self, key: LineKey, text: &str, to: &'static Lang, now: Instant) -> Option<Job>`
    - `pub fn incoming(&mut self, key: LineKey, text: &str, partner: &'static Lang, me: &'static Lang) -> Option<Job>` — `None` when `nothing_to_translate` or detected as `me` with confidence ≥ 0.9.
    - `pub fn done(&mut self, d: Done) -> Vec<Effect>` and `pub fn tick(&mut self, now: Instant) -> Vec<Effect>` (timeouts), where `pub enum Effect { Send { to: Uuid, text: String }, Replace { key: LineKey, text: String } }`. Outgoing `Send`s of one session are emitted in typed order: a finished later message waits until earlier ones of the same session are sent or failed.
    - `pub fn retry(&mut self, key: LineKey) -> Option<Job>`, `pub fn send_original(&mut self, key: LineKey) -> Option<Effect>`, `pub fn state(&self, key: &LineKey) -> Option<&LineState>`, `pub fn close_session(&mut self, s: Uuid)`, `pub fn clear(&mut self)`.
  - `pub struct Suggest` with `pub fn observe(&mut self, session: Uuid, text: &str, me: &'static Lang) -> Option<&'static Lang>` (rule in Global Constraints).

- [ ] **Step 1: Failing tests** (`translate/tests.rs`):
  - `outgoing_sends_the_translation`: `done(Ok("hola"))` → `[Send{"hola"}, Replace{"hola"}]`; state `Translated{original:"hello"}`.
  - `outgoing_failure_sends_nothing`: `Err` → no `Send`; state `Failed{Error}`; `send_original` → `Send{"hello"}`.
  - `outgoing_timeout`: `tick(now + 10.1s)` → `Failed{Timeout}`; a late `done` for it is ignored.
  - `too_long_is_held`: 1024-byte translation → `TooLong`, no `Send`.
  - `sends_keep_typed_order`: two outgoing in one session, second finishes first → its `Send` comes after the first's.
  - `results_for_a_closed_session_are_dropped`: `close_session` then `done` → no effects.
  - `incoming_in_my_language_is_not_translated` (me = en, `"I am looking for the shop near the beach"`) → `None`.
  - `suggests_after_two_spanish_messages`; `no_suggestion_after_short_or_mixed_messages`; `no_suggestion_once_dismissed` (dismissal is checked by the caller with `settings.translate.dismissed`; test that `observe` resets after a non-matching message).
  - `settings_round_trip`: `TranslateSettings` serializes and an old settings file without `translate` loads with defaults.
- [ ] **Step 2:** FAIL. **Step 3:** Implement. **Step 4:** PASS, commit `Add the conversation translation state`.

### Task 7: Wiring into the IM flow and `/tr`

**Files:** Modify `src/translate/mod.rs` (`TranslateService`), `src/world/mod.rs` (`own_im` returns `SystemTime`; `on_im` hook; `replace_line`), `src/world/transcripts.rs`, `src/app.rs` (IM loop at `for (to, mut text) in a.ims`, per-frame poll, logout), `src/ui/chat/compose.rs` (`/tr` before `ConvAction::Im`)

**Interfaces:**
- Consumes: Task 3, 4, 6.
- Produces:
  - `pub struct TranslateService { translations: Translations, suggest: Suggest, translator: Option<Translator>, download: Option<Download>, pub prompt_download: Option<Uuid /*session*/> }` with `pub fn partner(&self, settings, session) -> Option<&'static Lang>`, `pub fn submit_outgoing(...)`, `pub fn submit_incoming(...)`, `pub fn poll(&mut self, now) -> Vec<Effect>`, `pub fn set_partner(&mut self, settings, session, Option<&'static Lang>) -> SetResult` (`SetResult { Ready, NeedsDownload, NotBuilt }`), `pub fn me(&self, settings) -> &'static Lang` (falls back to the UI language, then `en`). The translator is spawned lazily on the first job with `backend_for(model_path)`; models directory `dirs::data_local_dir()/Aurora/AuroraViewer/data/models` (same root as `logging::logs_dir`).
  - `World::own_im(&mut self, to, text) -> SystemTime` (time of the pushed line); `World::replace_line(&mut self, key: &LineKey, text: &str)` sets `text` of the matching line and appends `"[translation] {text}"` to the transcript log (via `chat_log.append`, same `from`).
  - `World` exposes incoming private IMs to the app: push `(session, time, body)` onto `pub incoming_ims: Vec<(Uuid, SystemTime, String)>` in `on_im` right after `conversation_push` (body without the saved prefix); the app drains it every frame.
  - `pub enum TrCommand { Set(&'static Lang), Off, Status, Unknown(String) }`, `pub fn parse_tr(text: &str) -> Option<TrCommand>` (`/tr`, `/tr off`, `/tr <lang>`; anything not starting with `/tr` + space or end → `None`).

- [ ] **Step 1: Failing tests:** `parse_tr_forms` (`"/tr es"`, `"/tr Spanish"`, `"/tr off"`, `"/tr"`, `"/tr klingon"` → `Unknown`, `"/try"` → `None`); `saved_prefix_is_kept_out_of_the_job` (an offline IM pushes the body only into `incoming_ims`); `own_im_returns_the_line_time` (the returned time finds the line).
- [ ] **Step 2:** FAIL. **Step 3:** Implement:
  - compose: a `/tr` line in a private IM is not sent; it becomes `ConvAction::Translate { session, cmd }` handled in `app.rs` (set partner, open the download modal on `NeedsDownload`, system notice with the state for `Status`/`Unknown`/`NotBuilt`).
  - IM loop: after `expand_shortcodes`, for a non-group session with a partner: `let time = self.world.own_im(to, &text)`, `submit_outgoing`, `continue` (no `SendIm`). `/me` lines: translate the part after `/me ` and keep the prefix on `Send`.
  - Each frame: drain `world.incoming_ims` (submit for translated sessions, else feed `Suggest`), `poll` → `Effect::Send` → `NetCommand::SendIm`; `Effect::Replace` → `world.replace_line`.
  - Logout and session close: `clear` / `close_session`.
- [ ] **Step 4:** PASS; `cargo test --workspace --exclude aurora-render translate` green. Commit `Translate private IMs both ways`.

### Task 8: Chat UI, preferences and translations

**Files:** Modify `src/ui/chat.rs` (`line_extras`), `src/ui/chat/header.rs`, `src/ui/options/` (Chat page: new "Translation" section), `crates/aurora-i18n/locales/{en,fr,ja}.ftl`; Create `src/ui/chat/translate_card.rs`, `src/ui/translate_download.rs`; Modify `src/ui/theme_guard.rs` only if a new file needs an `ALLOWED` entry (it should not)

**Interfaces:**
- Consumes: `TranslateService::line_state(&LineKey) -> Option<&LineState>` (add it, delegating to `Translations::state`), `TranslateService::status() -> Option<Status>`, `TranslateService::download_state() -> Option<DownloadState>`.
- Produces: `ConvAction::Translate { session, cmd: TrCommand }`, `ConvAction::TranslateRetry(LineKey)`, `ConvAction::TranslateSendOriginal(LineKey)`, `ConvAction::TranslateEdit(LineKey)` (puts the original back in the input).

- [ ] **Step 1:** Line extras for a line with a state: `Pending` → dimmed text + `widgets` spinner + "Translating…"; `Translated` → original below in `p.muted_dim`, one size smaller, a click swaps the two; `Failed`/`TooLong` → `widgets::action_button` Retry / Send original / Edit.
- [ ] **Step 2:** Header pill "EN ⇄ ES" (Tabler `language` icon) in private IMs only; menu (`ui/menu.rs` rows): the 30 languages, Off, "Preview before sending" check (`settings.translate.preview`; with it, outgoing `Send` is held as a draft above the input: Enter sends, Esc cancels); tooltip "Running on CPU (slower)" when `!status.on_gpu`, "Translator offline" on `Closed`.
- [ ] **Step 3:** Suggestion card in the conversation: "They seem to write {language}. Translate this chat?" with `[EN ⇄ ES]` (primary) and `[Not now]` (adds to `dismissed`).
- [ ] **Step 4:** Download modal (`widgets::Modal`, `widgets::progress`): model label and size, Download / Cancel; errors in words (no space, network, checksum).
- [ ] **Step 5:** Preferences > Chat > Translation: my language (combo, "Same as the interface" first), model (catalogue labels with sizes, "Downloaded" mark, Delete), preview switch (`widgets::switch_labeled`).
- [ ] **Step 6:** All keys in the three catalogues (`translate-*`), following `docs/TRANSLATING.md` (Japanese: full-width units, no plural). Run `cargo test --workspace --exclude aurora-render theme_guard i18n` → PASS. Commit `Add the chat translation interface`.

### Task 9: Demo scenario and docs

**Files:** Create `src/demo_translate.rs`; Modify `src/demo.rs` (hook like `demo_keywords`), `docs/TESTING.md`, `ARCHITECTURE.md`, `TASKS.md`

**Interfaces:** Consumes `TranslateService` (a constructor `TranslateService::with_backend(Box<dyn Backend>)` for the demo), `FakeBackend`.

- [ ] **Step 1:** `AURORA_DEMO_TRANSLATE=1`: a demo resident (reuse `DEMO_TESS`) sends 2 Spanish IMs 1 s apart (suggestion card shows); `=on` starts with the session already set to `es` and sends 2 more after 3 s; `AURORA_TRANSLATE_REAL=1` uses the real backend and the downloaded default model instead of `FakeBackend`.
- [ ] **Step 2:** Captures: `AURORA_DEMO_TRANSLATE=1 AURORA_CAPTURE=.scratch/card.png AURORA_CAPTURE_EXIT=1 AURORA_CAPTURE_FRAMES=900 cargo run -p aurora-viewer -- --title "Test chat-translator"` and the same with `=on`; open the IM; inspect both images against the neighbouring conversation UI.
- [ ] **Step 3:** Docs: one table line per variable in `docs/TESTING.md` plus a Details paragraph; `ARCHITECTURE.md` (`translate/` module, threads section: the translation worker); `TASKS.md` 🔧 line "Local chat translator (fork experiment; task `chat-translator`)". Commit `Add the translation demo and docs`.

### Task 10: Verification and human test

- [ ] **Step 1:** `cargo fmt --check`; clippy as in Global Constraints; `cargo test --workspace --exclude aurora-render` → all green (give the counts).
- [ ] **Step 2:** `cargo build -p aurora-viewer --features translator`; run `AURORA_DEMO_TRANSLATE=on AURORA_TRANSLATE_REAL=1 AURORA_PROFILE=1`; record frame time idle vs while translating, tokens/s from the log, GPU/CPU backend.
- [ ] **Step 3:** Launch `cargo run -p aurora-viewer --features translator -- --title "Test chat-translator"` for the human: test the download modal, `/tr es`, the pill, both directions with the real model in the demo; then, on the grid (the human logs in), an IM with a person on another viewer.
- [ ] **Step 4:** After the human's validation: `TASKS.md` line to ✅ (or 🟡 for the grid part), push the branch to the fork (`git push -u origin feat/chat-translator`). No upstream PR.
