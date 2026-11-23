# Shadow regression tests

Run from the Aurora checkout with Python 3.10 or newer and a WebGPU-capable adapter.

```console
python -m pip install wgpu==0.33.0 pytest
python -m pytest -q docs/p3-rendering/qa/shadow-regression
```

Each test compiles rendering functions from the checkout's WGSL and reads back the GPU output. Use `-s` to print the measurements, `--source-root PATH` to select another checkout with these shader functions, and `--target CASE` to select one or more cases.

| Case | What the test checks |
| --- | --- |
| `cascade` | A thin post advancing through the transition between two directional shadow maps retains one shadow band. |
| `remote` | Combining local and remote depth layers preserves fractional coverage and avoids double-darkening an overlapping edge. |
| `projector` | The normal projector filter and optional soft-projector fallback retain a one-texel caster. |
| `blended` | The 50% alpha boundary matches for contact prepass and receivers. Ambient occlusion keeps its own opacity rule. |

These cases isolate shader behavior. Use the separate [bridge pullback](../bridge-pullback/README.md) for contact-filter behavior during camera motion and the [in-viewer matrix](../../VALIDATION.md) to compare actual content and frame time.
