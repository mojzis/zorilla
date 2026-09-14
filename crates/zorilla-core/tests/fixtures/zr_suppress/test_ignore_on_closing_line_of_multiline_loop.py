# This file is `ruff format` output (black agrees): each directive was
# written on the `for` line and the formatter moved it to the `):` line.


def test_for_header_split_across_lines(tmp_path):
    keep = tmp_path / "a"
    live = tmp_path / "b"
    for f in (
        keep,
        live,
    ):  # zorilla: ignore[ZR001] -- mixed database fixture, touches every path once
        f.touch()
    assert keep.exists()


def test_body_of_the_loop_is_not_covered(tmp_path):
    # The header's directive stops at the `:`; the literal in the body is
    # a separate statement and its ZR005 finding stays visible.
    for f in (
        tmp_path / "a",
    ):  # zorilla: ignore -- fixture setup, every code in the header
        f.write_text(open("/etc/hosts").read())
    assert True
