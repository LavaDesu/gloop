CREATE TABLE IF NOT EXISTS bets
(
    msg_id INTEGER PRIMARY KEY NOT NULL,
    start_time TEXT NOT NULL,
    stop_time TEXT,
    end_time TEXT,
    blue_win BOOL
);

CREATE TABLE IF NOT EXISTS bets_events
(
    id INTEGER PRIMARY KEY NOT NULL,
    discord_id INTEGER NOT NULL,
    target BOOLEAN NOT NULL,
    time TEXT NOT NULL,
    bet_placed INTEGER NOT NULL,
    bet INTEGER NOT NULL,
    FOREIGN KEY(bet) REFERENCES bets(msg_id)
);

