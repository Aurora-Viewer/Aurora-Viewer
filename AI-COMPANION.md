# Publish into your existing Red2Blaze/Aurora-Viewer fork

This kit never forks/creates a repository. It clones the **existing** fork, creates `feature/autonomous-avatar-ai` from `main`, applies the Rust patch, adds the Python agent and GitHub Actions workflow, and pushes **only** that feature branch. It does **not** open or merge a pull request.

Prerequisites: Windows, Python 3.11+, Git and GitHub CLI (`gh`) logged in as **Red2Blaze**. `gh auth status` should succeed. If git push complains about workflow permission, run `gh auth refresh -h github.com -s workflow` and retry the push inside the checkout shown by the script.

From the folder containing this file:

```powershell
py -3 .\publish_to_existing_fork.py
```

Use `py -3 .\publish_to_existing_fork.py --dry-run` to validate without touching GitHub. You can choose a different destination with `--workspace C:\Src\Aurora-AI`.

After pushing, open https://github.com/Red2Blaze/Aurora-Viewer/actions. Forks sometimes require manual confirmation to enable Actions: use the page's **Enable workflows** button if present. Wait for the workflow **Aurora AI - Windows build**, then open the run and download the artifact named `Aurora-AI-Viewer-windows-x64`. It contains the patched viewer, Aurora assets and the agent. **If Actions are disabled, no build starts until you enable them**.

The workflow uses GitHub-hosted Windows build minutes and artifact storage. Unlike a release, artifacts expire after 14 days. No release is automatically published and `main` remains untouched.

**Status:** Python tests pass locally; Rust compilation and in-world testing must be validated by the workflow and then in Second Life. The project requires a separate avatar flagged **Scripted Agent**. See `ai-companion/README.md` in the branch for runtime instructions. The viewer is GPL-3.0-or-later, and upstream notices and history are retained.
