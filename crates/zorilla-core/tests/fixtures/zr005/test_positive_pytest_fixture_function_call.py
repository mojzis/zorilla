import pytest


@pytest.fixture
def sample_url():
    # Inside a `@pytest.fixture` body — silenced by the case-4 carve-out.
    return "https://api.example.com/fixture"


def test_inline_url_still_fires():
    # Outside any fixture function — an inline literal URL inside a
    # regular `test_*` body must still fire. The case-4 carve-out is
    # scoped to fixture function bodies only and must not leak into
    # regular tests. This is the inline-test-body case from the
    # `projects` corpus that the brief explicitly forbids silencing.
    repo = build_repo(url="https://github.com/owner/repo")
    assert repo
