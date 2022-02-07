#!/bin/bash
TEST_DB="postgres://forktester:forktester@localhost/forktester"

# TODO: should work with cli tool on master....
#diesel migration --database-url  $TEST_DB revert all
diesel migration --database-url  $TEST_DB run
psql $TEST_DB -f ./nodes_setup.sql
