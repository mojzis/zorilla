import asyncio


async def test_waits_asyncio():
    await asyncio.sleep(0.1)
    assert True
