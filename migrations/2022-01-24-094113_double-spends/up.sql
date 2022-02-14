-- Your SQL goes here
CREATE TABLE stale_candidate (
	height bigint not null,
	n_children int not null,
	confirmed_in_one_branch_total float(53) not null,
	double_spent_in_one_branch_total float(53) not null,
	rbf_total float(53) not null,
	height_processed bigint,
	PRIMARY KEY(height)
);

CREATE TABLE rbf_by (
    candidate_height bigint not null,
	txid varchar not null,
	PRIMARY KEY(candidate_height, txid),
	CONSTRAINT fk_rbf_by
	    FOREIGN KEY(candidate_height)
		REFERENCES stale_candidate(height)
		ON DELETE CASCADE
);

CREATE TABLE double_spent_by (
    candidate_height bigint not null,
	txid varchar not null,
	PRIMARY KEY(candidate_height, txid),
	CONSTRAINT fk_rbf_by
	    FOREIGN KEY(candidate_height)
		REFERENCES stale_candidate(height)
		ON DELETE CASCADE
);

CREATE TABLE transaction (
	block_id varchar not null,
	txid varchar not null,
	is_coinbase boolean not null,
	hex varchar not null,
	amount float(53) not null,
	PRIMARY KEY(block_id, txid),
	CONSTRAINT fk_block_id
	    FOREIGN KEY(block_id)
		REFERENCES blocks(hash)
		ON DELETE CASCADE
);

CREATE TABLE stale_candidate_children (
    candidate_height bigint not null,
	root_id varchar not null,
	tip_id varchar not null,
	len int not null,
	PRIMARY KEY(root_id),
	CONSTRAINT fk_canditate_height
	    FOREIGN KEY(candidate_height)
		    REFERENCES stale_candidate(height),
	CONSTRAINT fk_tip_id
	    FOREIGN KEY(tip_id)
		    REFERENCES blocks(hash),
	CONSTRAINT fk_root_id
	    FOREIGN KEY(root_id)
		    REFERENCES blocks(hash)
);

ALTER TABLE blocks
ADD COLUMN work varchar not null;
