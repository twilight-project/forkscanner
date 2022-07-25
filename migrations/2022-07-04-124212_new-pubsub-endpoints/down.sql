-- This file should undo anything in `up.sql`
DROP TABLE lags;
DROP TABLE watched;

ALTER TABLE transaction
DROP COLUMN address;

ALTER TABLE invalid_blocks
DROP COLUMN created_at;

ALTER TABLE valid_blocks
DROP COLUMN created_at;
