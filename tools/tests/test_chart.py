"""chart.compute with no counted play returns an empty chart instead of raising."""
import pytest

from rormpc_tools import chart


@pytest.mark.parametrize("kind", ["quarter", "month"])
def test_no_events_gives_an_empty_chart(kind):
    data = chart.compute([], kind)
    assert data["buckets"] == [] and data["songs"] == [] and data["coverage"] == []
    assert data["sources"] == [] and data["events"] == 0


def test_only_unnamed_plays_give_an_empty_chart():
    events = [{"source": "mpd", "ts": "2024-05-01T10:00:00Z"}]  # a file name that resolves to no song
    data = chart.compute(events, "quarter")
    assert data["buckets"] == [] and data["events"] == 0
