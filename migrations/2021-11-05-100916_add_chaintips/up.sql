-- Your SQL goes here
CREATE TABLE nodes (
	id bigint generated always as identity,
	node varchar not null,
	rpc_host varchar not null,
	rpc_port int not null,
	mirror_rpc_host varchar not null,
	mirror_rpc_port int not null,
	rpc_user varchar not null,
	rpc_pass varchar not null,
	unreachable_since timestamp with time zone,
	PRIMARY KEY(id)
);

CREATE TABLE chaintips (
	id bigint generated always as identity,
	node bigint not null,
	status varchar not null,
	block varchar not null,
	height bigint not null,
	parent_chaintip bigint,
	PRIMARY KEY(id),
	CONSTRAINT fk_node
	  FOREIGN KEY(node)
	    REFERENCES nodes(id),
	CONSTRAINT fk_parent
	  FOREIGN KEY(parent_chaintip)
	    REFERENCES chaintips(id)
	    ON DELETE SET NULL
);

CREATE TABLE invalid_blocks (
	hash varchar not null,
	node bigint not null,
	PRIMARY KEY (hash, node),
	CONSTRAINT fk_hash
	  FOREIGN KEY(hash)
	    REFERENCES blocks(hash),
	CONSTRAINT fk_node
	  FOREIGN KEY(node)
	    REFERENCES nodes(id)
);

CREATE TABLE valid_blocks (
	hash varchar not null,
	node bigint not null,
	PRIMARY KEY (hash, node),
	CONSTRAINT fk_hash
	  FOREIGN KEY(hash)
	    REFERENCES blocks(hash),
	CONSTRAINT fk_node
	  FOREIGN KEY(node)
	    REFERENCES nodes(id)
);

ALTER TABLE blocks
ADD COLUMN node_id bigint not null CONSTRAINT fk_block_node REFERENCES nodes(id),
ADD COLUMN headers_only boolean not null default false;
