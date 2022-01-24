#!/bin/bash
TEST_DB="postgres://forktester:forktester@localhost/forktester"
diesel migration --database-url  $TEST_DB run
psql $TEST_DB -f ./nodes_setup.sql
