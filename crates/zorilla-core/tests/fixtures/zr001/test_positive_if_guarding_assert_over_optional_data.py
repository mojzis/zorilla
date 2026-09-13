def test_sparkline_ordering(points):
    if points:
        assert points == sorted(points)
