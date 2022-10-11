INSERT INTO chaintips
    (id, node, status, block, height, parent_chaintip)
VALUES
    (0, 0, 'active', '0000000000000000000501b978d69da3d476ada6a41aba60a426badbadbadbad', 5, NULL),
    (1, 1, 'active', '0000000000000000000501b978d69da3d476ada6a41aba60a42612806204013a', 5, NULL),
    (2, 1, 'valid-headers', '0000000000000000000f2eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee', 0, NULL),
    (3, 1, 'headers-only', '0000000000000000000eeeeee777777777777772222222222222222222222222', 0, NULL),
    (4, 2, 'active', '0000000000000000000aaaaaaaaaaaabbbbbbbbbbbcccccccccccccccccccccc', 70000000, NULL);
    (5, 3, 'active', '0000000000000000000aaaaaaaaaaaabbbbbbbbbbbcccccccccddddddddddddd', 0, 0);


INSERT  INTO blocks (hash, height, parent_hash, connected, first_seen_by, headers_only, work)
VALUES
    ('0000000000000000000501b978d69da3d476ada6a41aba60a42612806204013a', 10, '00000000000000000001ca4713bbb6900e61c6e3d6cbcbec958c0c580711afeb', true, 0, false, 0),
    ('00000000000000000001ca4713bbb6900e61c6e3d6cbcbec958c0c580711afeb', 9, '0000000000000000000000000000000000000000000000000000000000000000', false, 0, false, 0)
    ('0000000000000000000501b978d69da3d476ada6a41aba60a426badbadbadbad', 9, '0000000000000000000000000000000000000000000000000000000000000000', false, 0, false, 0)
	ON CONFLICT DO NOTHING;

INSERT INTO invalid_blocks (hash, id)
VALUES ('0000000000000000000501b978d69da3d476ada6a41aba60a426badbadbadbad', 1);
