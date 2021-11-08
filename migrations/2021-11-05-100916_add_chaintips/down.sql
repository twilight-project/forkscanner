-- This file should undo anything in `up.sql`
DROP TABLE chaintips;

ALTER TABLE blocks DROP COLUMN marked_valid;
ALTER TABLE blocks DROP COLUMN marked_invalid;
