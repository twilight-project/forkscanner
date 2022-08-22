-- This file should undo anything in `up.sql`
DROP TABLE lags;
DROP TABLE watched;

ALTER TABLE transaction
DROP COLUMN swept,
DROP COLUMN address;

ALTER TABLE invalid_blocks
DROP COLUMN created_at;

ALTER TABLE valid_blocks
DROP COLUMN created_at;

ALTER TABLE nodes
DROP COLUMN mirror_host,
DROP COLUMN mirror_last_polled,
DROP COLUMN mirror_unreachable_since,
DROP COLUMN archive;
