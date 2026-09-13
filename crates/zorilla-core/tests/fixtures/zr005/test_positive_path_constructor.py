from pathlib import Path


def test_literal_error_context():
    path = Path("/fictional/input.txt")
    assert str(path) == "/fictional/input.txt"
