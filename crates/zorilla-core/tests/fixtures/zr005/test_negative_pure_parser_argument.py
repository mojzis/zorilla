# The hosts resolve, so only the parse-only callee silences these.
from pathlib import PurePosixPath
from urllib.parse import urlparse


def test_url_parser():
    result = urlparse("https://example.com/project")
    assert result.path == "/project"


def test_pure_path():
    path = PurePosixPath("/fictional/input.txt")
    assert path.name == "input.txt"
