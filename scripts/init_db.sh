#!/bin/bash

DB_USERNAME=${DB_USERNAME:-forkscanner}
DB_PASS=${DB_PASS:-forkscanner}
DATABASE_URL="postgres://${DB_USERNAME}:${DB_PASS}@localhost/forkscanner"

psql $DATABASE_URL -c "CREATE USER $DB_USERNAME WITH ENCRYPTED PASSWORD '$DB_PASS'"
psql $DATABASE_URL -c "CREATE DATABASE forkscanner"
psql $DATABASE_URL -c "GRANT ALL PRIVILEGES ON DATABASE forkscanner TO $DB_USERNAME"

diesel migration run
