import json

import h3
import pytest

import ferrobus


def test_create_transit_point(model):
    lat, lon = 56.252619, 93.532134
    point = ferrobus.create_transit_point(lat, lon, model)
    assert point is not None
    assert hasattr(point, "coordinates")


def test_create_transit_point_invalid(model):
    lat, lon = 0.0, 0.0  # far from any data
    with pytest.raises(Exception):  # noqa: B017
        ferrobus.create_transit_point(lat, lon, model)


def test_calculate_isochrone(model):
    lat, lon = 56.25788847445582, 93.53960625054688
    point = ferrobus.create_transit_point(lat, lon, model)
    area_wkt = "POLYGON ((93.57274857628481 56.18357044999381, 93.57274857628481 56.30437667924404, 93.39795011002934 56.30437667924404, 93.39795011002934 56.18357044999381, 93.57274857628481 56.18357044999381))"  # noqa: E501
    index = ferrobus.create_isochrone_index(
        transit_model=model, area=area_wkt, cell_resolution=10
    )
    isochrone = ferrobus.calculate_isochrone(
        transit_model=model,
        start_point=point,
        departure_time=43200,
        max_transfers=3,
        cutoff=1200,
        index=index,
    )

    assert isinstance(isochrone, str)
    assert isochrone[0:18] == "MULTIPOLYGON(((93."


def test_travel_time_matrix(model):
    points = [
        ferrobus.create_transit_point(56.252619, 93.532134, model),
        ferrobus.create_transit_point(56.242574, 93.499159, model),
    ]
    matrix = ferrobus.travel_time_matrix(
        model, points, departure_time=8 * 3600, max_transfers=2
    )
    assert isinstance(matrix, list)
    assert len(matrix) == len(points)
    # Edge projections and off-street connectors are included in these costs.
    assert matrix[0] == [0, 1066]
    assert matrix[1] == [1278, 0]


def test_find_route(model):
    start_point = ferrobus.create_transit_point(56.256657, 93.533561, model)
    end_point = ferrobus.create_transit_point(56.242574, 93.499159, model)
    result = ferrobus.find_route(
        transit_model=model,
        start_point=start_point,
        end_point=end_point,
        departure_time=43200,
        max_transfers=2,
    )
    assert isinstance(result, dict)
    assert result["travel_time_seconds"] == 1644


def test_find_routes_one_to_many(model):
    start_point = ferrobus.create_transit_point(56.256657, 93.533561, model)
    end_points = [
        ferrobus.create_transit_point(56.242574, 93.499159, model),
        ferrobus.create_transit_point(56.231878, 93.552460, model),
    ]
    results = ferrobus.find_routes_one_to_many(
        transit_model=model,
        start_point=start_point,
        end_points=end_points,
        departure_time=43200,
        max_transfers=2,
    )
    assert isinstance(results, list)
    assert len(results) == len(end_points)
    for res in results:
        assert res is None or isinstance(res, dict)

    assert results[0]["travel_time_seconds"] == 1546
    assert results[1]["travel_time_seconds"] == 729


def test_transit_point_properties(model):
    point = ferrobus.create_transit_point(56.252619, 93.532134, model)
    coords = point.coordinates()
    assert isinstance(coords, tuple)
    assert len(coords) == 2
    assert all(isinstance(x, float) for x in coords)
    assert isinstance(point.nearest_stops(), list)

    assert isinstance(repr(point), str)


def test_range_multimodal_routing(model):
    start_point = ferrobus.create_transit_point(56.256657, 93.533561, model)
    end_point = ferrobus.create_transit_point(56.242574, 93.499159, model)
    result = ferrobus.range_multimodal_routing(
        transit_model=model,
        start_point=start_point,
        end_point=end_point,
        departure_range=(43200, 44000),
        max_transfers=2,
    )

    assert eval(result.__str__()) == {
        "journeys": [
            {
                "travel_time": 893,
                "transfers": 1,
                "walking_time": 82,
                "departure_time": 43951,
                "arrival_time": 44844,
            },
            {
                "travel_time": 1193,
                "transfers": 1,
                "walking_time": 82,
                "departure_time": 43651,
                "arrival_time": 44844,
            },
            {
                "travel_time": 1553,
                "transfers": 1,
                "walking_time": 82,
                "departure_time": 43291,
                "arrival_time": 44844,
            },
        ]
    }


def test_pareto_range_multimodal_routing(model):
    start_point = ferrobus.create_transit_point(56.256657, 93.533561, model)
    end_point = ferrobus.create_transit_point(56.242574, 93.499159, model)
    result = ferrobus.pareto_range_multimodal_routing(
        transit_model=model,
        start_point=start_point,
        end_point=end_point,
        departure_range=(43200, 44000),
        max_transfers=2,
    )

    assert eval(result.__str__()) == {
        "journeys": [
            {
                "travel_time": 893,
                "transfers": 1,
                "walking_time": 82,
                "departure_time": 43951,
                "arrival_time": 44844,
            }
        ]
    }


def test_detailed_journey(model):
    start_point = ferrobus.create_transit_point(
        56.256657,
        93.533561,
        transit_model=model,
    )
    end_point = ferrobus.create_transit_point(56.231878, 93.552460, transit_model=model)

    result = ferrobus.detailed_journey(
        transit_model=model,
        start_point=start_point,
        end_point=end_point,
        departure_time=43200,  # Время отправления (12:00)
        max_transfers=3,
    )

    assert isinstance(result, str)

    geojson = json.loads(result)
    if len(geojson["features"]) == 3:
        access_leg, transit_leg, egress_leg = geojson["features"]

        assert access_leg["properties"] == {
            "arrival_time": 43223,
            "departure_time": 43200,
            "duration": 23,
            "from_name": "",
            "leg_type": "access_walk",
            "to_name": "21",
        }

        assert transit_leg["properties"] == {
            "arrival_time": 43920,
            "departure_time": 43320,
            "duration": 600,
            "from_name": "21",
            "leg_index": 0,
            "leg_type": "transit",
            "route_id": "bus_9",
            "to_name": "74",
            "trip_id": "bus_9_dir0_11_53_winter_weekday",
        }

        assert egress_leg["properties"] == {
            "arrival_time": 43935,
            "departure_time": 43920,
            "duration": 15,
            "from_name": "74",
            "leg_type": "egress_walk",
            "to_name": "",
        }


def test_reachable_cells(model):
    lat, lon = 56.25788847445582, 93.53960625054688
    point = ferrobus.create_transit_point(lat, lon, model)
    area_wkt = "POLYGON ((93.57274857628481 56.18357044999381, 93.57274857628481 56.30437667924404, 93.39795011002934 56.30437667924404, 93.39795011002934 56.18357044999381, 93.57274857628481 56.18357044999381))"  # noqa: E501
    index = ferrobus.create_isochrone_index(
        transit_model=model, area=area_wkt, cell_resolution=8
    )

    def reached(cutoff):
        return set(
            ferrobus.reachable_cells(
                transit_model=model,
                start_point=point,
                departure_time=43200,
                max_transfers=2,
                cutoff=cutoff,
                index=index,
            )
        )

    near, far = reached(900), reached(2700)
    grid = set(index.cells())

    assert near, "a 15 minute cutoff should reach something"
    assert all(len(c) == 15 and int(c, 16) for c in near)

    assert near <= grid
    assert far <= grid
    assert near < far, "the cutoff made no difference"
    assert far < grid, "a 45 minute cutoff reached the whole index"


def test_reachable_cells_are_readable_by_h3(model):
    """The reason these are strings: h3-py should take them without conversion."""
    lat, lon = 56.25788847445582, 93.53960625054688
    point = ferrobus.create_transit_point(lat, lon, model)
    area_wkt = "POLYGON ((93.57274857628481 56.18357044999381, 93.57274857628481 56.30437667924404, 93.39795011002934 56.30437667924404, 93.39795011002934 56.18357044999381, 93.57274857628481 56.18357044999381))"  # noqa: E501
    index = ferrobus.create_isochrone_index(
        transit_model=model, area=area_wkt, cell_resolution=8
    )

    cells = ferrobus.reachable_cells(
        transit_model=model,
        start_point=point,
        departure_time=43200,
        max_transfers=2,
        cutoff=1800,
        index=index,
    )

    assert cells
    assert all(h3.is_valid_cell(cell) for cell in cells)
    assert {h3.get_resolution(cell) for cell in cells} == {8}
