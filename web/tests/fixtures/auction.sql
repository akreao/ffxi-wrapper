-- Auction house rows for tests/mariadb.rs (load after public.sql). Pipiru (10005) and
-- Torvald (10001) trade with the bot, which writes seller 0 and, when it buys, buyer 0.
INSERT INTO auction_house (itemid, stack, seller, seller_name, date, price, buyer, buyer_name, sale, sell_date) VALUES
-- Fire crystals: two singles and a stack up, two singles sold.
(4096, 0, 10005, 'Pipiru', 1763000000, 300, 0, NULL, 0, 0),
(4096, 0, 0, 'M.H.M.U.', 1763000100, 250, 0, NULL, 0, 0),
(4096, 1, 0, 'M.H.M.U.', 1763000100, 3000, 0, NULL, 0, 0),
(4096, 0, 10005, 'Pipiru', 1762000000, 280, 0, 'M.H.M.U.', 280, 1762000500),
(4096, 0, 0, 'M.H.M.U.', 1762000000, 260, 10001, 'Torvald', 260, 1762100000),
-- A bronze sword up; distilled water only sold (nothing up now).
(16535, 0, 10001, 'Torvald', 1763000000, 1200, 0, NULL, 0, 0),
(4509, 0, 0, 'M.H.M.U.', 1762000000, 40, 0, 'M.H.M.U.', 40, 1762000000),
-- Items players can't have, which must never show: the Judge's shield (GM only) and gil.
(12332, 0, 0, 'M.H.M.U.', 1763000000, 1, 0, NULL, 0, 0),
(65535, 0, 10005, 'Pipiru', 1763000000, 1, 0, NULL, 0, 0);
