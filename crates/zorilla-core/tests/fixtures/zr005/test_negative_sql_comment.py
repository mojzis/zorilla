def test_query_uses_sql_comment():
    # The string contains a SQL comment that incidentally starts with "/*".
    # ZR005 must not flag SQL comment literals as a mystery-guest path.
    sql = "/* warm up cache */ SELECT 1"
    result = db.execute(sql)
    assert result
