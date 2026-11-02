# Second Life Autonomous AI Character — v0.1 (experimental)

This is an original Python starter for a **local-first autonomous Second Life avatar**, with SQLite memory, independent rotating goals and carefully bounded self-suggested curiosity goals, Lumen LEAP integration, and an Ollama multimodal brain. It is **not** a finished Tessel clone.

## What works

- **Simulated demo:** local model observes a fake garden, chooses to journal/wait/wander, remembers across runs. No Second Life account necessary.
- **Live viewer:** when launched as a Lumen LEAP subprocess it reads current avatar position, counts nearby avatars and objects, takes a viewer screenshot, runs a local multimodal model, and can optionally take **short bounded autopilot walks**.
- **Privacy:** the adapter does not intentionally pass avatar names to the model; however, screenshots and model-written journal entries can contain visible names or other personal information. Review the private database before sharing it. Journal and goals stay in `data/character.sqlite3`, screenshots are temporary files deleted when each decision completes or fails. No remote LLM endpoint is allowed in v0.1.
- **Controls:** `pause`, `resume`, `stop`, `status`, `add-goal` commands. 10 decisions per live session by default. Set `-Steps 0` to continue until stopped.

**Not implemented yet:** actual nearby chat/IM reception or sending, object building, conversational memory for other residents, safe pathfinding, teleport scheduling, task supervision UI. These require additional viewer integration. The `--scripted` option tests the loop, not artificial intelligence.

## Before going in-world

1. Create a **separate Second Life avatar account**, log in manually, choose an outfit, and mark it as a **Scripted Agent** at https://secondlife.com/my/account/sisa.php in its Linden Lab account settings. Use a region that permits scripted agents. Do NOT enter credentials into this Python project.
2. Install **Lumen** from https://github.com/cattencarter/lumen/releases . It is experimental and not supported by Firestorm or Linden Lab.
3. Install **Python 3.11+** and **Ollama** (https://ollama.com/download/windows). Run `ollama pull qwen3-vl:4b-instruct` in a terminal. This small vision model is a starting point for RTX 3070 hardware. If it is slow, try `qwen3-vl:2b` and update config.json.
4. Unzip this project to a folder **without spaces**, e.g. `C:\SLAI`, because Lumen LEAP cannot start subprocess commands with spaces in paths (as documented by its author).

## Run simulated mode first

Double-click `run_scripted_test.bat` to check Python and SQLite without needing the model or Second Life.

Then double-click `run_demo.bat` to use your **actual locally running Ollama model** in a fake garden. The agent journal and choices are saved in `data/character.sqlite3`.

Try:

```powershell
cd C:\SLAI
.\.venv\Scripts\python.exe companion.py status
.\.venv\Scripts\python.exe companion.py add-goal "Notice three details in my surroundings"
```

## Connect to the Second Life avatar

Open PowerShell in the project folder. If you used the setup above:

```powershell
.\start_lumen.ps1 -LumenExe 'C:\Program Files\Lumen\Lumen.exe' -Steps 10
```

The script starts Lumen with the agent as a LEAP child. **Log the viewer into the new scripted-agent avatar manually.** This starter never captures or stores a password. Once logged in, the agent collects screenshots, counts nearby objects/avatars and makes a decision every 15 seconds (after inference).

LEAP logs are shown in `%APPDATA%\Lumen\logs\Lumen.log`; because LEAP requires protocol-only stdout, the agent writes logs to stderr. The UI may not show its own console.

**Walking is disabled by default**. In `config.json`, set `"allow_walking": true` only when testing on land you own or have permission to use, away from crowds. Walk commands are limited to 6 meters and an autopilot stop is sent within 3 seconds. Autopilot does *not* guarantee the destination is safe, reachable or reached. No flight, teleport or chat is available in this version.

## Stop / pause / inspect

In another PowerShell window:

```powershell
.\.venv\Scripts\python.exe companion.py pause
.\.venv\Scripts\python.exe companion.py resume
.\.venv\Scripts\python.exe companion.py status
.\.venv\Scripts\python.exe companion.py stop
```

Stop is checked between steps and again before any action is applied. It cannot interrupt an active Ollama inference call, but discards a pending action once the call finishes. You can also close the viewer, or disable walking in `config.json` **before starting a new session**. `resume` clears the STOP marker.

## Customize

- `personality.txt`: editable character identity and interests.
- `config.json`: model, wait interval, screenshots and walk permission. `config.json` is read only when the agent starts.
- `add-goal`: append independent goals. Current goals rotate in creation/last-use order. The AI can suggest up to 12 total low-impact curiosity goals, which you can review in `status`.

## Architecture

```
Lumen viewer (separate scripted-agent avatar)
     | LEAP stdin/stdout (binary / notation LLSD)
     v
companion.py -> observe -> Ollama Qwen3-VL (localhost)
     |               |
     |               -> screenshot (temporary local file)
     -> SQLite journal + goals + action log
     -> local safety filter -> optional LLAgent autopilot
```

The LEAP operations are documented at https://github.com/cattencarter/lumen/blob/ai-control/LEAP.md . The available operations vary by viewer version; v0.1 is built against the published 0.1.8 list, and **live mode has not been tested against an actual running Lumen installation** in this environment. If it throws an API error, check the Lumen log for the request and viewer-version differences.

The browser's `http://127.0.0.1:11434/api/chat` is a local endpoint; do not configure Ollama to listen on a public interface. Model weights may have their own license. Avoid recording or publishing residents' messages or images without their consent.

## Developer tests

```powershell
.\.venv\Scripts\python.exe -m unittest discover -s tests -v
```

## Next milestones

- v0.2: viewer bridge for chat receive/send (with consent and rate limits) and names, without UI scraping.
- v0.3: explicit trusted destinations, navigation guardrails and map memory.
- v0.4: bounded LSL building and self-review with owner approval.
- v0.5: dedicated desktop supervision panel, profile editor, schedules, and support for stronger optional local/cloud models.
