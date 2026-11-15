-- Fixture for tests/items.rs: a hand-written main.lua; zz_app.lua (loaded later) wins.
xi = xi or {}
xi.settings = xi.settings or {}

xi.settings.main =
{
    -- MAX_LEVEL = 10, a comment, not a setting
    NORMAL_MOB_MAX_LEVEL_RANGE_MAX = 30,
    MAX_LEVEL = 60,
}
