"""Run the production shader regressions for the current or a specified checkout."""

from pathlib import Path
import subprocess
import sys


def test_shadow_regression(shadow_case, common_shader):
    scripts = {
        "cascade": "cascade-handoff.py",
        "remote": "remote-coverage.py",
        "projector": "projector.py",
        "blended": "blend-depth.py",
    }
    probe = Path(__file__).with_name(scripts[shadow_case])
    shader = (
        common_shader.with_name("object.wgsl")
        if shadow_case == "blended"
        else common_shader
    )
    result = subprocess.run(
        [sys.executable, str(probe), str(shader)], text=True, capture_output=True
    )
    assert result.returncode == 0, result.stdout + result.stderr
    print(result.stdout)
