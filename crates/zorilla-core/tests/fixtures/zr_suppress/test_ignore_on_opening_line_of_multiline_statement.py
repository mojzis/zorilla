import time


def test_directive_on_the_opening_line_reaches_an_inner_literal():
    response = client.fetch(  # zorilla: ignore[ZR005] -- local double
        "https://api.example.com/v1/things",
        timeout=5,
    )
    assert response.ok


def test_directive_on_the_opening_line_does_not_reach_the_next_statement():
    values = [  # zorilla: ignore -- literal table
        "https://api.example.com/v1/things",
    ]
    time.sleep(1)
    assert values
