-- This file should undo anything in `up.sql`
DROP TABLE inflated_blocks;
DROP TABLE tx_outsets;
DROP TABLE pool;
DROP TABLE fee_rates;
DROP TABLE block_templates;

ALTER TABLE nodes
DROP COLUMN last_polled,
DROP COLUMN initial_block_download;

ALTER TABLE blocks
DROP COLUMN txids,
DROP COLUMN pool_name,
DROP COLUMN template_txs_fee_diff,
DROP COLUMN total_fee,
DROP COLUMN txids_added,
DROP COLUMN txids_omitted,
DROP COLUMN tx_omitted_fee_rates,
DROP COLUMN lowest_template_fee_rate,
DROP COLUMN coinbase_message;
