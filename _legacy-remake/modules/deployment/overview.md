# deployment — Domain Overview

## Responsibility
System deployment and initialization: Docker Compose for MariaDB and InfluxDB, database schema initialization, migration scripts, and system bootstrap.

## Legacy Source
- docker/docker-compose.yml
- docker/init.sql
- docker/mariadb/Dockerfile, docker/influxdb/Dockerfile

## Key Business Rules
- MariaDB: schema creation, seed data for sensor types
- InfluxDB: setup-mode with org=fitc, bucket=iotkit, admin token
- Node-RED runs on host OS (not containerized)
- Hardware access (serial, I2C, GPIO) requires host-level access

## Dependencies
- device-config-service (schema definitions)
- timeseries-service (InfluxDB setup)
