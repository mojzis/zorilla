from contextlib import nullcontext


def test_closing_an_already_closed_stream_is_safe():
    import io

    stream = io.StringIO()
    stream.close()
    with nullcontext():
        stream.close()
