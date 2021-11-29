-- This file should undo anything in `up.sql`

ALTER TABLE blocks
DROP COLUMN node_id,
DROP COLUMN headers_only;

DROP TABLE valid_blocks;
DROP TABLE invalid_blocks;
DROP TABLE chaintips;
DROP TABLE nodes;
