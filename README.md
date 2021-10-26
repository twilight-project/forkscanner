# Setting up

## Postgres
```
CREATE USER forkscanner WITH ENCRYPTED PASSWORD 'forkscanner';
CREATE DATABASE forkscanner;
GRANT ALL PRIVILEGES ON DATABASE forkscanner TO forkscanner;
```

Do the above with user forktester as well to run the tests.

## Install diesel cli-tool
`cargo install diesel_cli --no-default-features --features postgres`

`diesel migration run`

## Test program
This needs to be run on a node with bitcoin running.
`cargo run`
