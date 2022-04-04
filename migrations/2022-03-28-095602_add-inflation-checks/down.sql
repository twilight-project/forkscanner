-- This file should undo anything in `up.sql`
DROP TABLE inflated_blocks;
DROP TABLE tx_outsets;
DROP TABLE pool;
DROP TABLE block_templates;
DROP TABLE fee_rates;

ALTER TABLE nodes
DROP COLUMN last_polled,
DROP COLUMN initial_block_download;

ALTER TABLE blocks
DROP COLUMN txids,
DROP COLUMN pool_name,
DROP COLUMN total_fee,
DROP COLUMN coinbase_message;
