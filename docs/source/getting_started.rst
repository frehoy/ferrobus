Getting Started
===============

Ferrobus is easily installable from PyPI and supports Windows, Linux, and macOS.

Pre-built binaries are available for **Windows (x86_64), macOS (Apple Silicon), and Linux (x86_64, arm64)**.
Wheels for Linux are available for musl-based systems (e.g., Alpine Linux) and manylinux2014-compliant systems.

Supported Python versions are **CPython 3.9 and later, including free-threaded 3.13t and PyPy >3.11**

Installation
------------

To install Ferrobus, run:

.. code-block:: bash

   pip install ferrobus

This will install pre-built binaries for all major platforms from PyPI.

Quick Usage
-----------

Here's a simple example to get you started:

.. code-block:: python

   import ferrobus
   import datetime

   # Create a transit model from GTFS and OSM data
   model = ferrobus.create_transit_model(
       osm_path="path/to/city.osm.pbf", # OSM pbf file, preferably filtered by region and tag
       gtfs_dirs=["path/to/gtfs_data", "path/to/another_gtfs"], # feeds, operating in the same region
       date=datetime.date.today() # date to use when filtering GTFS data / None for all dates
   )

   # Create origin and destination points
   origin = ferrobus.create_transit_point(
       lat=59.85,
       lon=30.22,
       transit_model=model
   )

   destination = ferrobus.create_transit_point(
       lat=59.97,
       lon=30.50,
       transit_model=model
   )

   # Find the optimal route at noon (12:00)
   route = ferrobus.find_route(
       transit_model=model,
       start_point=origin,
       end_point=destination,
       departure_time=12*3600,  # 12:00 noon in seconds since midnight
       max_transfers=3
   )

   # Print the results
   print(f"Travel time: {route['travel_time_seconds'] / 60:.1f} minutes")
    print(f"Number of transfers: {route['transfers']}")

Building the Model Once
-----------------------

Creating the model parses the OSM extract and the GTFS feeds, which dominates the
cost of a short script. If you run more than once, build it once and save it:

.. code-block:: python

   # Once, in a preparation step
   model = ferrobus.create_transit_model(
       osm_path="path/to/city.osm.pbf",
       gtfs_dirs=["path/to/gtfs_data"],
       date=datetime.date.today(),
   )
   ferrobus.save_transit_model(model, "city.ferrobus")

   # In every later run
   model = ferrobus.load_transit_model("city.ferrobus")

Or let ferrobus handle both sides for you — this builds and caches the model on
the first call, and loads the cache on every call after that:

.. code-block:: python

   model = ferrobus.load_or_create_transit_model(
       cache_path="city.ferrobus",
       osm_path="path/to/city.osm.pbf",
       gtfs_dirs=["path/to/gtfs_data"],
       date=datetime.date.today(),
   )

For detailed examples, see the :doc:`demo` notebook and the :doc:`ferrobus` API documentation.
