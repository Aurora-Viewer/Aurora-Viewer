# Aurora AI Companion v0.2 — local-first autonomous Second Life avatar

**Experimental source package, not a pre-built viewer.** This package is a native Rust bridge patch for [Aurora Viewer](https://github.com/Aurora-Viewer/Aurora-Viewer) plus a Python/Ollama agent with SQLite journal and goals. It is not Lumen and does **not** use LEAP.

## Source and status

Patch designed from Aurora `main` as inspected 2026-11-02. Files targeted:

- `crates/aurora-viewer/src/app.rs` (AI control input applied after autopilot update)
- New `crates/aurora-viewer/src/app/ai_control.rs`

The original upstream code is **not** modified remotely; the GitHub connection has read-only permission to `Aurora-Viewer/Aurora-Viewer`. The patch script changes only your **local checkout**, makes `.ai-backup` copies, and refuses to install if its exact anchors have changed. **No Windows Aurora compilation or live-grid validation was possible here.** Python tests passed in the packaging environment. Use the Aurora build instructions for your platform: see its README and `scripts/` folder. Aurora's status is early development; some features may be unfinished.

## Architecture

```
Second Life grid <--> Aurora Viewer (Rust, your scripted-agent account)
                             |  native world/chat/movement
                    ai_control.rs (opt-in)
                             |  JSON lines, localhost 127.0.0.1:57833
                             |  32-byte random per-launch authentication token
                       Python aurora_agent.py
                             |--> local Ollama Qwen3-VL
                             |--> SQLite memory/goals
                             '--> viewer-window screenshot, optional
```

The native bridge never sees or relays your SL password. It does not expose general-purpose browser, filesystem, LSL execution, inventory, payment, teleport, or arbitrary scripting endpoints.

## 1. Install prerequisites

- Windows 11, Visual Studio C++ build tools as required by Aurora, Rust stable with Cargo, Git, Python 3.11+, optional `mss` (installed by launcher).
- Ollama: https://ollama.com/download/windows
- Download model: `ollama pull qwen3-vl:4b-instruct` (smaller option: `qwen3-vl:2b`). The RTX 3070 8GB may need a smaller model or reduced other GPU load.
- Separate Second Life account marked **Scripted Agent** under Linden Lab policy, with permission to visit the test region. Log in **manually** using Aurora.

## 2. Patch and build your Aurora checkout

Open PowerShell:

```powershell
git clone --filter=blob:none https://github.com/Aurora-Viewer/Aurora-Viewer.git C:\Src\Aurora-Viewer
py -3 C:\Path\To\aurora-ai-companion-v0.2\aurora_patch\apply_patch.py C:\Src\Aurora-Viewer
cd C:\Src\Aurora-Viewer
cargo fmt --all
cargo check -p aurora-viewer
cargo build -p aurora-viewer
```

Replace `C:\Path\To` with the folder you extracted. For any missing build tools, follow [Aurora README / Build](https://github.com/Aurora-Viewer/Aurora-Viewer#build) and [Aurora's scripts](https://github.com/Aurora-Viewer/Aurora-Viewer/tree/main/scripts). **Do not run `cargo fmt --all` on a shared working copy unless you are comfortable with other source being reformatted.** You can use `rustfmt` on the new file if preferred. After building, you should have `C:\Src\Aurora-Viewer\target\debug\aurora-viewer.exe` (if standard Cargo dev output is used; verify actual output). This compiled **patched** binary is required; the official release executable won't have the AI bridge.

**Important:** Aurora development changes quickly. If the patch refuses an anchor, do not force it: update the patch against your exact commit. Backups are `app.rs.ai-backup`.

## 3. Test connection before enabling autonomy

```powershell
cd C:\Path\To\aurora-ai-companion-v0.2
.\start_aurora_ai.ps1 -AuroraExe 'C:\Src\Aurora-Viewer\target\debug\aurora-viewer.exe' -Scripted -Steps 3
```

Log into the **separate scripted-agent avatar** when Aurora opens. The scripted test journals a few steps without Ollama and without movement or chat. No login data is put in Python. If you're already using an Aurora instance, close it before starting the patched build, so the launcher starts the intended process.

Next, use your real local AI:

```powershell
.\start_aurora_ai.ps1 -AuroraExe 'C:\Src\Aurora-Viewer\target\debug\aurora-viewer.exe' -Steps 10
```

This launches an **ephemeral authentication token** into both processes and sets the viewer PID, so screenshots are limited to that Aurora window. Both processes run on your own PC. `-Steps 0` enables continuous running. Interrupt with Ctrl+C. Local Ollama runs on `127.0.0.1:11434`.

## 4. Enable only the capabilities you want

Open `agent/config.json`:

```json
{
  "name": "Mira",
  "model": "qwen3-vl:4b-instruct",
  "ollama_url": "http://127.0.0.1:11434/api/chat",
  "seconds_between_steps": 15,
  "screenshot": true,
  "allow_walking": false,
  "allow_chat": false
}
```

- First test with both actions **false**. Then enable walking and try a quiet, permitted region. Each movement is max **1.2 seconds** and automatically expires; **no collision avoidance or navigation guarantee** exists. Human keyboard input takes priority.
- To allow nearby chat replies, enable `allow_chat`. The Rust bridge adds its own 15-second rate limit, 180-char maximum, and normal-channel-only restriction. Python additionally permits AI replies only to **new messages explicitly containing the character's configured name**, not unsolicited spam. Chat observation is enabled by the launcher; the model may see up to eight recent local-chat lines and their names. They stay local but may be recorded in logs if you modify the software. Ask people before storing conversations as long-term social memory.
- For screenshot perception, the launcher passes the precise process ID to the Python capture. It crops the Aurora window using `mss` and resizes it to at most 1280×720 with Pillow. A minimized window cannot be captured reliably. Screenshots are sent to **local** Ollama and deleted afterward. To disable screenshot processing, set `screenshot:false`.

## Control and memory

Inside `agent/`, with the virtual environment created by the launcher:

```powershell
.\.venv\Scripts\python.exe companion.py status
.\.venv\Scripts\python.exe companion.py add-goal "Notice interesting buildings"
.\.venv\Scripts\python.exe companion.py pause
.\.venv\Scripts\python.exe companion.py resume
.\.venv\Scripts\python.exe companion.py stop
```

These utilities work with `data/character.sqlite3`. Use `agent/personality.txt` to customize the character. The old `companion.py` **demo** commands still simulate a scene; live Aurora sessions use **`aurora_agent.py`**, not `companion.py live`, which remains Lumen-specific legacy code. The launched viewer may remain open after the Python agent exits, but every movement command expires quickly.

## Development / test

```powershell
cd C:\Path\To\aurora-ai-companion-v0.2\agent
py -3 -m unittest discover -s tests -v
```

The source patch uses existing Aurora fields for `world.agent.position`, `world.chat`, `world.objects`, and `NetCommand::Chat`, and patches `app.rs` to poll incoming commands from the main thread and to apply movement to the real avatar input. The bridge thread never touches Aurora game state directly.

## Roadmap

1. Verify clean Rust compile and live connection against your Aurora commit.
2. Improve observation from avatar/object names, geometry, camera, spatial distance, pathfinding, and snapshot integration.
3. Add explicit consent/privacy controls for social memory, reply threading and avatar recognition.
4. Add safe teleport and object creation, then LSL writing in a sandboxed trial region with owner confirmation.
5. Supervisor UI, autonomous schedule and session handoff, optional cloud-model fallback.

## Limitations and safeguards

- **Not tested live** on Aurora or the SL grid; an Aurora build may expose integration issues. The package does not claim it has working building, complex navigation or a human-like autonomous social life.
- Do not open the bridge port to your LAN or the internet; its binding is hardcoded to `127.0.0.1`. A local malicious program running under your user context could potentially interact with other local processes; this is not a hardened security boundary against malware.
- Chat can be untrusted text. This agent always applies hard-coded action allowlists and refuses requests to transfer currency or alter inventory even when the model suggests them.
- Aurora is GPL-3.0-or-later; modifications and redistribution need to respect its licence and attributions. This package contains only new source and small patch instructions, not the Aurora source tree.
