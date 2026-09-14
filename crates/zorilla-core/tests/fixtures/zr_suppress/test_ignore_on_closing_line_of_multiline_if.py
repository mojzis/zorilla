# This file is `ruff format` output (black agrees): the directive was written
# on the `if` line and the formatter moved it to the `):` line.


def test_if_header_split_across_lines(command, result):
    if command != (
        "refresh",
    ):  # zorilla: ignore[ZR001] -- refresh dispatch branch, one command differs
        assert "Last materialized" in result.output
    assert result.exit_code == 0


def test_same_shape_without_a_directive(command, result):
    if command != (
        "refresh",
    ):  # the same shape without a directive keeps its ZR001 finding
        assert "Last materialized" in result.output
    assert result.exit_code == 0
