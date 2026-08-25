# ruff: noqa: B017

import datetime

import pytest

import ferrobus


def test_save_and_load_transit_model(model, tmp_path):
    path = str(tmp_path / "model.ferrobus")

    ferrobus.save_transit_model(model, path)
    loaded = ferrobus.load_transit_model(path)

    assert loaded.stop_count() == model.stop_count()
    assert loaded.route_count() == model.route_count()
    assert str(loaded) == str(model)
    assert loaded.feeds_info() == model.feeds_info()


def test_loaded_model_routes_identically(model, tmp_path):
    path = str(tmp_path / "model.ferrobus")
    ferrobus.save_transit_model(model, path)
    loaded = ferrobus.load_transit_model(path)

    departure_time = 12 * 3600
    origin_before = ferrobus.create_transit_point(56.256657, 93.533561, model)
    dest_before = ferrobus.create_transit_point(56.242574, 93.499159, model)
    origin_after = ferrobus.create_transit_point(56.256657, 93.533561, loaded)
    dest_after = ferrobus.create_transit_point(56.242574, 93.499159, loaded)

    before = ferrobus.find_route(
        transit_model=model,
        start_point=origin_before,
        end_point=dest_before,
        departure_time=departure_time,
        max_transfers=2,
    )
    after = ferrobus.find_route(
        transit_model=loaded,
        start_point=origin_after,
        end_point=dest_after,
        departure_time=departure_time,
        max_transfers=2,
    )

    assert after == before


def test_load_or_create_builds_then_reuses_cache(osm_path, gtfs_dirs, tmp_path):
    cache_path = tmp_path / "cache.ferrobus"

    built = ferrobus.load_or_create_transit_model(
        cache_path=str(cache_path),
        osm_path=osm_path,
        gtfs_dirs=gtfs_dirs,
        date=datetime.date(2024, 1, 11),
        max_transfer_time=600,
    )
    assert cache_path.exists()

    cached = ferrobus.load_or_create_transit_model(
        cache_path=str(cache_path),
        osm_path=osm_path,
        gtfs_dirs=gtfs_dirs,
        date=datetime.date(2024, 1, 11),
        max_transfer_time=600,
    )

    assert str(cached) == str(built)


def test_save_and_load_isochrone_index(model, isochrone_index, tmp_path):
    path = str(tmp_path / "index.ferrobus")

    ferrobus.save_isochrone_index(isochrone_index, path)
    loaded = ferrobus.load_isochrone_index(path)

    assert loaded.len() == isochrone_index.len()
    assert loaded.resolution() == isochrone_index.resolution()

    origin = ferrobus.create_transit_point(56.256657, 93.533561, model)
    before = ferrobus.calculate_isochrone(
        model, origin, 12 * 3600, 2, 1800, isochrone_index
    )
    after = ferrobus.calculate_isochrone(model, origin, 12 * 3600, 2, 1800, loaded)

    # The dissolve step does not pin which vertex a ring starts at, so compare
    # the vertex sets rather than the WKT verbatim.
    assert _vertex_set(after) == _vertex_set(before)


def test_load_missing_file_raises(tmp_path):
    with pytest.raises(Exception):
        ferrobus.load_transit_model(str(tmp_path / "does_not_exist.ferrobus"))


def test_load_foreign_file_raises(tmp_path):
    path = tmp_path / "foreign.ferrobus"
    path.write_bytes(b"this is definitely not a transit model")

    with pytest.raises(Exception):
        ferrobus.load_transit_model(str(path))


def test_load_wrong_artifact_kind_raises(isochrone_index, tmp_path):
    path = str(tmp_path / "index.ferrobus")
    ferrobus.save_isochrone_index(isochrone_index, path)

    with pytest.raises(Exception):
        ferrobus.load_transit_model(path)


def _vertex_set(wkt):
    """Coordinates of a MULTIPOLYGON WKT string, order-independent.

    Deduplicated, because each ring repeats its first vertex to close itself and
    which vertex that is shifts with the ring's rotation.
    """
    numbers = wkt[wkt.index("(") :].replace("(", " ").replace(")", " ")
    pairs = {chunk.strip() for chunk in numbers.split(",")}
    return sorted(pair for pair in pairs if pair)
