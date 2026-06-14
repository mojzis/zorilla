import pytest


@pytest.fixture
def sample_url():
    # Inside a `@pytest.fixture` body — silenced by the case-4 carve-out.
    return "https://api.example.com/fixture"


def test_inline_url_kwarg_carved_out():
    # Outside any fixture function — but the `url=` kwarg is in URL_KWARG_NAMES,
    # so carve-out 1c silences it. The prior version of this fixture expected
    # ZR005 to fire here; carve-out 1c intentionally changes that behavior.
    # See test_positive_kwarg_url_still_fires.py for a case that still fires
    # (string value in a dict whose key "x" is not in URL_KWARG_NAMES,
    # passed as a `headers` arg — neither the kwarg name nor the dict key
    # is in the carve-out set, so ZR005 fires).
    repo = build_repo(url="https://github.com/owner/repo")
    assert repo
