# Setting up

## Postgres
CREATE USER forkscanner WITH ENCRYPTED PASSWORD 'forkscanner';
CREATE DATABASE forkscanner;
GRANT ALL PRIVILEGES ON DATABASE forkscanner TO forkscanner;

## Install diesel cli-tool
`cargo install diesel_cli --no-default-features --features postgres`

`diesel migration run`
