ALTER TABLE peer RENAME TO peer_old;
UPDATE peer_old SET status = 1;
CREATE TABLE IF NOT EXISTS peer (
                                    guid blob primary key not null,
                                    id varchar(100) not null,
                                    uuid blob not null,
                                    pk blob not null,
                                    created_at datetime not null default(current_timestamp),
                                    "user" blob,
                                    status tinyint not null default(1),
                                    note varchar(300),
                                    region text null,
                                    strategy blob,
                                    info JSON not null DEFAULT '{}', 
                                    "last_online" datetime not null default('2011-11-16 11:55:19')
                                ) without rowid;
INSERT INTO peer(guid, id, uuid, pk, created_at, user, status, note, info) SELECT guid, id, uuid, pk, created_at, user, status, note, info FROM peer_old;
DROP TABLE peer_old;
