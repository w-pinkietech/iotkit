# timeseries-service — Domain Overview

## Responsibility
Time-series data storage (InfluxDB) and query aggregation. Handles 4 measurement write families and sensor log query/chart data shaping.

## Legacy Source
- センサーログ tab (66 nodes): time-series query, chart data shaping
- InfluxDB write nodes scattered across sensor-ingest paths

## Key Business Rules
- 4 write families: base measurement, <measurement>_count, spectrogram, pulse_count
- Measurements named from sensor_types.measurement, tagged by deviceName
- Query grouping by tag field
- InfluxDB org: fitc, bucket: iotkit

## Dependencies
- core-domain (measurement/device types)

## Downstream Consumers
- api-service (log query endpoints)
- ui-web (chart data)
