-- Your SQL goes here
CREATE TABLE inflated_blocks (
    block_hash varchar primary key,
	max_inflation decimal not null,
	actual_inflation decimal not null,
	notified_at timestamp with time zone not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	node_id bigint not null,
	dismissed_at timestamp with time zone ,
	CONSTRAINT fk_inflated_blocks_hash
	  FOREIGN KEY(block_hash)
	    REFERENCES blocks(hash)
		ON DELETE CASCADE
);

CREATE TABLE tx_outsets (
    block_hash varchar not null,
	node_id bigint not null,
	txouts bigint not null,
	total_amount decimal not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	inflated boolean not null,
	PRIMARY KEY (block_hash, node_id),
	CONSTRAINT fk_tx_outsets_block_hash
	  FOREIGN KEY(block_hash)
	    REFERENCES blocks(hash)
);

CREATE TABLE pool (
    tag varchar not null,
	name varchar not null,
	url varchar not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	PRIMARY KEY (tag, name, url)
);

CREATE TABLE block_templates (
    parent_block_hash varchar not null,
	node_id bigint not null,
	fee_total decimal not null,
	ts timestamp with time zone not null,
	height bigint not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	n_transactions integer not null,
	tx_ids bytea not null,
	lowest_fee_rate integer not null,
	PRIMARY KEY (parent_block_hash, node_id)
);

CREATE TABLE fee_rates (
    parent_block_hash varchar not null,
	node_id bigint not null,
	fee_rate integer not null,
	omitted boolean not null,
	PRIMARY KEY (parent_block_hash, node_id, fee_rate),
	CONSTRAINT fx_fee_rate_block_template
		FOREIGN KEY(parent_block_hash, node_id)
		    REFERENCES block_templates(parent_block_hash, node_id)
			ON DELETE CASCADE
);

CREATE TABLE softforks (
	node_id bigint not null,
	fork_type integer not null,
	name varchar not null,
	bit integer,
	status integer not null,
	since bigint,
	notified_at timestamp with time zone not null,
	created_at timestamp with time zone not null,
	updated_at timestamp with time zone not null,
	PRIMARY KEY (node_id, fork_type, name),
	CONSTRAINT fk_softfork_node_id
	    FOREIGN KEY(node_id)
		REFERENCES nodes(id)
);

ALTER TABLE blocks
ADD COLUMN txids bytea,
ADD COLUMN txids_added bytea,
ADD COLUMN txids_omitted bytea,
ADD COLUMN pool_name varchar,
ADD COLUMN template_txs_fee_diff decimal,
ADD COLUMN tx_omitted_fee_rates decimal,
ADD COLUMN lowest_template_fee_rate decimal,
ADD COLUMN total_fee decimal,
ADD COLUMN coinbase_message bytea;

ALTER TABLE nodes
ADD COLUMN last_polled timestamp with time zone;

ALTER TABLE nodes
ADD COLUMN initial_block_download boolean not null default true;
