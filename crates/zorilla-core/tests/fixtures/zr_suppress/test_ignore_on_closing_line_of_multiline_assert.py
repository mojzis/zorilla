# This file is `ruff format` output (black agrees). The directives below
# were written on the anchor line, over the width limit; the formatter split
# each statement and moved the comment to the closing bracket's line.
import time


def test_directive_moved_to_the_closing_paren_line(output, popen_calls, argv):
    # ZR004 reports at the `assert` keyword; one directive anywhere in the
    # statement covers it.
    assert (
        "Server ready" in output
    )  # zorilla: ignore[ZR004] -- startup CLI contract, five subjects
    assert len(popen_calls) == 1
    assert "-m" in argv
    assert "-p" in argv
    assert "-q" in argv
    time.sleep(1)
