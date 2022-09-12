-- This file should undo anything in `up.sql`
DROP TABLE lags;
DROP TABLE watched;
DROP TABLE transaction_addresses;

ALTER TABLE transaction
DROP COLUMN swept;

ALTER TABLE invalid_blocks
DROP COLUMN created_at;

ALTER TABLE valid_blocks
DROP COLUMN created_at;

ALTER TABLE nodes
DROP COLUMN mirror_host,
DROP COLUMN mirror_last_polled,
DROP COLUMN mirror_unreachable_since,
DROP COLUMN archive;
