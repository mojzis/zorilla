import pytest


@pytest.fixture
def sample_pr():
    # A fixture function returning a dataclass with a URL — the literal
    # is fixture data, not a request target. The fixture function itself
    # is also not iterated by `iter_test_functions` (its name doesn't
    # start with `test_`), so this file is mostly a documentation /
    # belt-and-braces fixture: it pins that strings inside a
    # `@pytest.fixture` body never fire ZR005, even if iteration is
    # later widened.
    return PullRequest(url="https://github.com/owner/repo/pull/42")


class TestRepository:
    @pytest.fixture
    def sample_repo(self):
        # Class-level `@pytest.fixture` method — same carve-out applies.
        # `sample_repo` does start with `s` not `test_`, so it is also
        # excluded by `iter_test_functions`, but the helper must still
        # treat any string in its body as fixture data.
        return Repo(url="https://github.com/owner/repo")
