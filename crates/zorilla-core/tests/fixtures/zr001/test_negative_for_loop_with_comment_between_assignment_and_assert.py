def test_for_loop_with_inline_comment():
    xs = [1, 2, 3]
    for x in xs:
        result = transform(x)
        # explanation of what we are asserting on
        assert result in xs


def test_for_loop_with_only_comment_and_assert():
    xs = [1, 2, 3]
    for x in xs:
        # describe the assertion
        assert x in xs
