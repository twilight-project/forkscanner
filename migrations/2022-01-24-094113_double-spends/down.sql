-- This file should undo anything in `up.sql`
DROP TABLE rbf_by;
DROP TABLE double_spent_by;
DROP TABLE stale_candidate_children;
DROP TABLE stale_candidate;
DROP TABLE transaction;

ALTER TABLE blocks
DROP COLUMN work;
