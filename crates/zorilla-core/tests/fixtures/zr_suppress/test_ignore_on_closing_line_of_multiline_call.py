# This file is `ruff format` output (black agrees): the directive was written
# on the call's line and the formatter moved it to the closing paren's line.


def test_literal_on_an_inner_line_of_the_call(client):
    # ZR005 anchors on the literal's own line, not the call's first line.
    response = client.fetch(
        "https://api.example.com/v1/things", timeout=5, retries=3
    )  # zorilla: ignore[ZR005] -- local double
    assert response.ok


def test_literal_in_the_same_call_shape_without_a_directive(client):
    response = client.fetch(
        "https://api.example.com/v1/things", timeout=5, retries=3, verify=False
    )
    assert response.ok
