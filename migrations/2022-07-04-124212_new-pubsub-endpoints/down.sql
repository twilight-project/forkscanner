-- This file should undo anything in `up.sql`
DROP TABLE lags;

ALTER TABLE invalid_blocks
DROP COLUMN created_at;

ALTER TABLE valid_blocks
DROP COLUMN created_at;
