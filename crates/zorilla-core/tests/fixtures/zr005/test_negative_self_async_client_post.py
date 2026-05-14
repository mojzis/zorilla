class TestUsers:
    async def test_creates(self):
        resp = await self.async_client.post("/users", json={"name": "a"})
        assert resp.ok
