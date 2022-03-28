-- This file should undo anything in `up.sql`
DROP TABLE inflated_blocks;
DROP TABLE tx_outsets;

ALTER TABLE nodes
DROP COLUMN last_polled,
DROP COLUMN initial_block_download;
