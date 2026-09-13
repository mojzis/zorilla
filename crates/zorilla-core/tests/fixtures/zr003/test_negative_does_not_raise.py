from contextlib import nullcontext as does_not_raise


def test_closing_an_already_closed_stream_is_safe():
    import io

    stream = io.StringIO()
    stream.close()
    with does_not_raise():
        stream.close()
