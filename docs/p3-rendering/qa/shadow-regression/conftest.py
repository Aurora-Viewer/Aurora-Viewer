from pathlib import Path

import pytest


CASES = ("cascade", "remote", "projector", "blended")


def pytest_addoption(parser):
    group = parser.getgroup("Aurora shadow regressions")
    group.addoption(
        "--source-root",
        type=Path,
        default=Path(__file__).resolve().parents[4],
        help="Aurora viewer checkout to test.",
    )
    group.addoption(
        "--target",
        choices=CASES,
        action="append",
        default=[],
        help="Only run this case; can be repeated.",
    )


def pytest_generate_tests(metafunc):
    if "shadow_case" in metafunc.fixturenames:
        metafunc.parametrize(
            "shadow_case", metafunc.config.getoption("--target") or CASES
        )


@pytest.fixture(scope="session")
def common_shader(pytestconfig):
    path = (
        pytestconfig.getoption("--source-root").resolve()
        / "crates/aurora-render/src/shaders/common.wgsl"
    )
    if not path.is_file():
        pytest.fail(
            f"Pass --source-root with the Aurora viewer checkout. Missing {path}"
        )
    return path
